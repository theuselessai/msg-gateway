//! File Cache Module
//!
//! Handles downloading, caching, and serving files for inbound/outbound messages.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::config::FileCacheConfig;
use crate::error::AppError;

/// Metadata for a cached file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFile {
    pub file_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub created_at: u64, // Unix timestamp
    pub path: PathBuf,
}

/// File cache manager
pub struct FileCache {
    config: FileCacheConfig,
    /// In-memory index of cached files
    files: RwLock<HashMap<String, CachedFile>>,
    /// Base URL for download links
    base_url: String,
}

impl FileCache {
    /// Create a new file cache
    pub async fn new(config: FileCacheConfig, gateway_url: &str) -> Result<Self, AppError> {
        // Ensure cache directory exists
        fs::create_dir_all(&config.directory)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create cache directory: {}", e)))?;

        let cache = Self {
            config,
            files: RwLock::new(HashMap::new()),
            base_url: gateway_url.to_string(),
        };

        // Load existing files from disk
        cache.scan_directory().await?;

        Ok(cache)
    }

    /// Scan cache directory and rebuild index
    async fn scan_directory(&self) -> Result<(), AppError> {
        let dir = Path::new(&self.config.directory);

        let mut entries = fs::read_dir(dir)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read cache directory: {}", e)))?;

        let mut files = self.files.write().await;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read directory entry: {}", e)))?
        {
            let path = entry.path();

            // Look for .meta files
            if path.extension().map(|e| e == "meta").unwrap_or(false)
                && let Ok(content) = fs::read_to_string(&path).await
                && let Ok(cached) = serde_json::from_str::<CachedFile>(&content)
            {
                files.insert(cached.file_id.clone(), cached);
            }
        }

        tracing::info!(cached_files = files.len(), "File cache index loaded");

        Ok(())
    }

    /// Generate a unique file ID
    fn generate_file_id() -> String {
        format!(
            "f_{}",
            &uuid::Uuid::new_v4().to_string().replace("-", "")[..12]
        )
    }

    /// Validate MIME type against config
    fn validate_mime_type(&self, mime_type: &str) -> Result<(), AppError> {
        // Check blocked list first
        for blocked in &self.config.blocked_mime_types {
            if mime_matches(mime_type, blocked) {
                return Err(AppError::Internal(format!(
                    "MIME type {} is blocked",
                    mime_type
                )));
            }
        }

        // If allowed list is non-empty, check against it
        if !self.config.allowed_mime_types.is_empty() {
            let allowed = self
                .config
                .allowed_mime_types
                .iter()
                .any(|pattern| mime_matches(mime_type, pattern));
            if !allowed {
                return Err(AppError::Internal(format!(
                    "MIME type {} is not in allowed list",
                    mime_type
                )));
            }
        }

        Ok(())
    }

    /// Download a file from URL and cache it
    pub async fn download_and_cache(
        &self,
        url: &str,
        auth_header: Option<&str>,
        filename: &str,
        mime_type: &str,
    ) -> Result<CachedFile, AppError> {
        // Validate MIME type
        self.validate_mime_type(mime_type)?;

        let file_id = Self::generate_file_id();
        let max_size = (self.config.max_file_size_mb as u64) * 1024 * 1024;

        // Download file
        let client = reqwest::Client::new();
        let mut request = client.get(url);

        if let Some(auth) = auth_header {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to download file: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Internal(format!(
                "File download failed: {}",
                response.status()
            )));
        }

        // Check content length if available
        if let Some(content_length) = response.content_length()
            && content_length > max_size
        {
            return Err(AppError::Internal(format!(
                "File too large: {} bytes (max {} MB)",
                content_length, self.config.max_file_size_mb
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read file content: {}", e)))?;

        if bytes.len() as u64 > max_size {
            return Err(AppError::Internal(format!(
                "File too large: {} bytes (max {} MB)",
                bytes.len(),
                self.config.max_file_size_mb
            )));
        }

        // Determine file extension
        let ext = Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");

        // Save file
        let file_path = PathBuf::from(&self.config.directory).join(format!("{}.{}", file_id, ext));

        let mut file = fs::File::create(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to create cache file: {}", e)))?;

        file.write_all(&bytes)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write cache file: {}", e)))?;

        // Create metadata
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cached = CachedFile {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            size_bytes: bytes.len() as u64,
            created_at: now,
            path: file_path.clone(),
        };

        // Save metadata
        let meta_path = PathBuf::from(&self.config.directory).join(format!("{}.meta", file_id));
        let meta_json = serde_json::to_string_pretty(&cached).unwrap();
        fs::write(&meta_path, meta_json)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write metadata: {}", e)))?;

        // Add to index
        {
            let mut files = self.files.write().await;
            files.insert(file_id.clone(), cached.clone());
        }

        tracing::info!(
            file_id = %file_id,
            filename = %filename,
            size = bytes.len(),
            "File cached"
        );

        Ok(cached)
    }

    /// Store file data directly (for outbound files from backend)
    #[allow(dead_code)]
    pub async fn store_file(
        &self,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
    ) -> Result<CachedFile, AppError> {
        self.validate_mime_type(mime_type)?;

        let max_size = (self.config.max_file_size_mb as u64) * 1024 * 1024;
        if data.len() as u64 > max_size {
            return Err(AppError::Internal(format!(
                "File too large: {} bytes (max {} MB)",
                data.len(),
                self.config.max_file_size_mb
            )));
        }

        let file_id = Self::generate_file_id();

        // Determine file extension
        let ext = Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");

        // Save file
        let file_path = PathBuf::from(&self.config.directory).join(format!("{}.{}", file_id, ext));

        fs::write(&file_path, &data)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

        // Create metadata
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cached = CachedFile {
            file_id: file_id.clone(),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            size_bytes: data.len() as u64,
            created_at: now,
            path: file_path.clone(),
        };

        // Save metadata
        let meta_path = PathBuf::from(&self.config.directory).join(format!("{}.meta", file_id));
        let meta_json = serde_json::to_string_pretty(&cached).unwrap();
        fs::write(&meta_path, meta_json)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write metadata: {}", e)))?;

        // Add to index
        {
            let mut files = self.files.write().await;
            files.insert(file_id.clone(), cached.clone());
        }

        tracing::info!(
            file_id = %file_id,
            filename = %filename,
            size = data.len(),
            "File stored"
        );

        Ok(cached)
    }

    /// Get a cached file by ID
    pub async fn get(&self, file_id: &str) -> Option<CachedFile> {
        let files = self.files.read().await;
        files.get(file_id).cloned()
    }

    /// Read file content
    pub async fn read_file(&self, file_id: &str) -> Result<Vec<u8>, AppError> {
        let cached = self
            .get(file_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("File not found: {}", file_id)))?;

        // Check if file is expired
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let ttl_secs = (self.config.ttl_hours as u64) * 3600;
        if now - cached.created_at > ttl_secs {
            return Err(AppError::Gone(format!("File expired: {}", file_id)));
        }

        fs::read(&cached.path)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to read file: {}", e)))
    }

    /// Get download URL for a cached file
    pub fn get_download_url(&self, file_id: &str) -> String {
        format!("{}/files/{}", self.base_url, file_id)
    }

    /// Get file path (for passing to adapters)
    #[allow(dead_code)]
    pub async fn get_file_path(&self, file_id: &str) -> Option<PathBuf> {
        let files = self.files.read().await;
        files.get(file_id).map(|f| f.path.clone())
    }

    /// Delete a cached file
    #[allow(dead_code)]
    pub async fn delete(&self, file_id: &str) -> Result<(), AppError> {
        let cached = {
            let mut files = self.files.write().await;
            files.remove(file_id)
        };

        if let Some(cached) = cached {
            // Delete file and metadata
            let _ = fs::remove_file(&cached.path).await;
            let meta_path = PathBuf::from(&self.config.directory).join(format!("{}.meta", file_id));
            let _ = fs::remove_file(&meta_path).await;

            tracing::debug!(file_id = %file_id, "File deleted");
        }

        Ok(())
    }

    /// Run cleanup to remove expired files
    #[allow(dead_code)]
    pub async fn cleanup(&self) -> Result<usize, AppError> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let ttl_secs = (self.config.ttl_hours as u64) * 3600;
        let mut removed = 0;

        let expired: Vec<String> = {
            let files = self.files.read().await;
            files
                .iter()
                .filter(|(_, f)| now - f.created_at > ttl_secs)
                .map(|(id, _)| id.clone())
                .collect()
        };

        for file_id in expired {
            if self.delete(&file_id).await.is_ok() {
                removed += 1;
            }
        }

        if removed > 0 {
            tracing::info!(removed = removed, "Cleaned up expired files");
        }

        Ok(removed)
    }

    /// Get cache statistics
    #[allow(dead_code)]
    pub async fn stats(&self) -> FileCacheStats {
        let files = self.files.read().await;
        let total_bytes: u64 = files.values().map(|f| f.size_bytes).sum();

        FileCacheStats {
            file_count: files.len(),
            total_bytes,
            max_bytes: (self.config.max_cache_size_mb as u64) * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct FileCacheStats {
    pub file_count: usize,
    pub total_bytes: u64,
    pub max_bytes: u64,
}

/// Check if a MIME type matches a pattern (supports wildcards like "image/*")
fn mime_matches(mime_type: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern == "*/*" {
        return true;
    }

    if let Some(prefix) = pattern.strip_suffix("/*") {
        return mime_type.starts_with(prefix);
    }

    mime_type == pattern
}

/// Start background cleanup task
#[allow(dead_code)]
pub async fn start_cleanup_task(cache: Arc<FileCache>, interval_minutes: u32) {
    let interval = Duration::from_secs((interval_minutes as u64) * 60);

    tracing::info!(
        interval_minutes = interval_minutes,
        "Starting file cache cleanup task"
    );

    loop {
        tokio::time::sleep(interval).await;

        if let Err(e) = cache.cleanup().await {
            tracing::error!(error = %e, "File cache cleanup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_matches() {
        assert!(mime_matches("image/png", "image/*"));
        assert!(mime_matches("image/jpeg", "image/*"));
        assert!(!mime_matches("text/plain", "image/*"));
        assert!(mime_matches("text/plain", "text/plain"));
        assert!(mime_matches("anything", "*"));
        assert!(mime_matches("anything/here", "*/*"));
    }
}
