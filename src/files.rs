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
    use crate::config::FileCacheConfig;

    fn test_config(dir: &str) -> FileCacheConfig {
        FileCacheConfig {
            directory: dir.to_string(),
            max_file_size_mb: 10,
            max_cache_size_mb: 100,
            ttl_hours: 24,
            cleanup_interval_minutes: 60,
            allowed_mime_types: vec!["*/*".to_string()],
            blocked_mime_types: vec![],
        }
    }

    #[test]
    fn test_mime_matches() {
        assert!(mime_matches("image/png", "image/*"));
        assert!(mime_matches("image/jpeg", "image/*"));
        assert!(!mime_matches("text/plain", "image/*"));
        assert!(mime_matches("text/plain", "text/plain"));
        assert!(mime_matches("anything", "*"));
        assert!(mime_matches("anything/here", "*/*"));
    }

    #[test]
    fn test_generate_file_id() {
        let id1 = FileCache::generate_file_id();
        let id2 = FileCache::generate_file_id();

        assert!(id1.starts_with("f_"));
        assert_eq!(id1.len(), 14); // "f_" + 12 chars
        assert_ne!(id1, id2); // Should be unique
    }

    #[tokio::test]
    async fn test_file_cache_new() {
        let temp_dir = std::env::temp_dir().join("test_file_cache_new");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080").await;

        assert!(cache.is_ok());
        assert!(temp_dir.exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_store_and_get_file() {
        let temp_dir = std::env::temp_dir().join("test_store_get_file");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file
        let data = b"Hello, World!".to_vec();
        let cached = cache
            .store_file(data.clone(), "test.txt", "text/plain")
            .await
            .unwrap();

        assert!(cached.file_id.starts_with("f_"));
        assert_eq!(cached.filename, "test.txt");
        assert_eq!(cached.mime_type, "text/plain");
        assert_eq!(cached.size_bytes, 13);

        // Get the file back
        let retrieved = cache.get(&cached.file_id).await;
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.filename, "test.txt");

        // Get download URL
        let url = cache.get_download_url(&cached.file_id);
        assert!(url.contains(&cached.file_id));
        assert!(url.starts_with("http://localhost:8080/files/"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_delete_file() {
        let temp_dir = std::env::temp_dir().join("test_delete_file");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file
        let data = b"Delete me".to_vec();
        let cached = cache
            .store_file(data, "delete.txt", "text/plain")
            .await
            .unwrap();

        // Verify it exists
        assert!(cache.get(&cached.file_id).await.is_some());

        // Delete it
        cache.delete(&cached.file_id).await.unwrap();

        // Verify it's gone
        assert!(cache.get(&cached.file_id).await.is_none());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_validate_mime_type() {
        let temp_dir = std::env::temp_dir().join("test_validate_mime");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Config with blocked types
        let mut config = test_config(temp_dir.to_str().unwrap());
        config.blocked_mime_types = vec!["application/x-executable".to_string()];
        config.allowed_mime_types = vec!["image/*".to_string(), "text/*".to_string()];

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Should pass - image type is allowed
        assert!(cache.validate_mime_type("image/png").is_ok());

        // Should pass - text type is allowed
        assert!(cache.validate_mime_type("text/plain").is_ok());

        // Should fail - blocked type
        assert!(
            cache
                .validate_mime_type("application/x-executable")
                .is_err()
        );

        // Should fail - not in allowed list
        assert!(cache.validate_mime_type("video/mp4").is_err());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_file_too_large() {
        let temp_dir = std::env::temp_dir().join("test_file_too_large");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let mut config = test_config(temp_dir.to_str().unwrap());
        config.max_file_size_mb = 1; // 1 MB limit

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Create data larger than limit (1.5 MB)
        let data = vec![0u8; 1024 * 1024 + 512 * 1024];
        let result = cache
            .store_file(data, "large.bin", "application/octet-stream")
            .await;

        assert!(result.is_err());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_stats() {
        let temp_dir = std::env::temp_dir().join("test_file_stats");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Empty cache
        let stats = cache.stats().await;
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.total_bytes, 0);

        // Store some files
        cache
            .store_file(b"file1".to_vec(), "f1.txt", "text/plain")
            .await
            .unwrap();
        cache
            .store_file(b"file2 longer".to_vec(), "f2.txt", "text/plain")
            .await
            .unwrap();

        let stats = cache.stats().await;
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.total_bytes, 5 + 12);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let temp_dir = std::env::temp_dir().join("test_get_nonexistent");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        assert!(cache.get("nonexistent").await.is_none());
        // get_download_url always returns a URL regardless of whether file exists
        let url = cache.get_download_url("nonexistent");
        assert!(url.contains("nonexistent"));
        assert!(cache.get_file_path("nonexistent").await.is_none());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_read_file() {
        let temp_dir = std::env::temp_dir().join("test_read_file");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file
        let data = b"Hello, World!".to_vec();
        let cached = cache
            .store_file(data.clone(), "test.txt", "text/plain")
            .await
            .unwrap();

        // Read the file back
        let content = cache.read_file(&cached.file_id).await.unwrap();
        assert_eq!(content, data);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let temp_dir = std::env::temp_dir().join("test_read_file_not_found");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Try to read a non-existent file
        let result = cache.read_file("nonexistent").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::AppError::NotFound(_)));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_read_expired_file() {
        let temp_dir = std::env::temp_dir().join("test_read_expired_file");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create config with 24 hour TTL
        let config = test_config(temp_dir.to_str().unwrap());

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file
        let data = b"This will expire".to_vec();
        let cached = cache
            .store_file(data, "expire.txt", "text/plain")
            .await
            .unwrap();

        // Manually make the file appear expired by setting created_at to epoch
        {
            let mut files = cache.files.write().await;
            if let Some(f) = files.get_mut(&cached.file_id) {
                f.created_at = 0; // Epoch time = definitely expired
            }
        }

        // Try to read - should fail due to expiration
        let result = cache.read_file(&cached.file_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::AppError::Gone(_)));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cleanup_expired_files() {
        let temp_dir = std::env::temp_dir().join("test_cleanup_expired");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create config with 1 hour TTL
        let config = test_config(temp_dir.to_str().unwrap());

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store some files
        let cached1 = cache
            .store_file(b"file1".to_vec(), "f1.txt", "text/plain")
            .await
            .unwrap();
        let cached2 = cache
            .store_file(b"file2".to_vec(), "f2.txt", "text/plain")
            .await
            .unwrap();

        let stats_before = cache.stats().await;
        assert_eq!(stats_before.file_count, 2);

        // Manually make the files appear expired by modifying their created_at
        // in the in-memory index to be more than 24 hours ago
        {
            let mut files = cache.files.write().await;
            if let Some(f) = files.get_mut(&cached1.file_id) {
                f.created_at = 0; // Epoch time = definitely expired
            }
            if let Some(f) = files.get_mut(&cached2.file_id) {
                f.created_at = 0;
            }
        }

        // Run cleanup
        let removed = cache.cleanup().await.unwrap();
        assert_eq!(removed, 2);

        let stats_after = cache.stats().await;
        assert_eq!(stats_after.file_count, 0);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cleanup_no_expired_files() {
        let temp_dir = std::env::temp_dir().join("test_cleanup_no_expired");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Create config with long TTL
        let config = test_config(temp_dir.to_str().unwrap());

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store some files
        cache
            .store_file(b"file1".to_vec(), "f1.txt", "text/plain")
            .await
            .unwrap();

        // Run cleanup - should not remove any files
        let removed = cache.cleanup().await.unwrap();
        assert_eq!(removed, 0);

        let stats = cache.stats().await;
        assert_eq!(stats.file_count, 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_get_file_path() {
        let temp_dir = std::env::temp_dir().join("test_get_file_path");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file
        let cached = cache
            .store_file(b"test".to_vec(), "test.txt", "text/plain")
            .await
            .unwrap();

        // Get file path
        let path = cache.get_file_path(&cached.file_id).await;
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.exists());
        assert!(path.to_str().unwrap().contains(&cached.file_id));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_scan_directory_on_startup() {
        let temp_dir = std::env::temp_dir().join("test_scan_directory");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // First, create a cache and store a file
        {
            let config = test_config(temp_dir.to_str().unwrap());
            let cache = FileCache::new(config, "http://localhost:8080")
                .await
                .unwrap();

            cache
                .store_file(b"persistent".to_vec(), "persist.txt", "text/plain")
                .await
                .unwrap();

            let stats = cache.stats().await;
            assert_eq!(stats.file_count, 1);
        }
        // Cache dropped here, but files remain on disk

        // Create a new cache instance - should scan and find the file
        {
            let config = test_config(temp_dir.to_str().unwrap());
            let cache = FileCache::new(config, "http://localhost:8080")
                .await
                .unwrap();

            let stats = cache.stats().await;
            assert_eq!(stats.file_count, 1);
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_file() {
        let temp_dir = std::env::temp_dir().join("test_delete_nonexistent");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Deleting a non-existent file should succeed (no-op)
        let result = cache.delete("nonexistent").await;
        assert!(result.is_ok());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_store_file_without_extension() {
        let temp_dir = std::env::temp_dir().join("test_store_no_ext");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let config = test_config(temp_dir.to_str().unwrap());
        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Store a file without extension
        let cached = cache
            .store_file(b"binary data".to_vec(), "noextension", "application/octet-stream")
            .await
            .unwrap();

        // Should default to .bin extension
        assert!(cached.path.to_str().unwrap().ends_with(".bin"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_mime_matches_edge_cases() {
        // Empty pattern should not match
        assert!(!mime_matches("image/png", ""));

        // Partial prefix without wildcard should not match
        assert!(!mime_matches("image/png", "image"));

        // Different types should not match
        assert!(!mime_matches("audio/mp3", "video/*"));

        // Exact match works
        assert!(mime_matches("application/json", "application/json"));
    }

    #[tokio::test]
    async fn test_validate_empty_allowed_list() {
        let temp_dir = std::env::temp_dir().join("test_validate_empty_allowed");
        let _ = std::fs::remove_dir_all(&temp_dir);

        // Config with empty allowed list (should allow everything not blocked)
        let mut config = test_config(temp_dir.to_str().unwrap());
        config.allowed_mime_types = vec![];
        config.blocked_mime_types = vec!["application/x-malware".to_string()];

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        // Any type should be allowed (except blocked)
        assert!(cache.validate_mime_type("image/png").is_ok());
        assert!(cache.validate_mime_type("video/mp4").is_ok());
        assert!(cache.validate_mime_type("application/x-malware").is_err());

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_cached_file_serialization() {
        use std::path::PathBuf;

        let cached = CachedFile {
            file_id: "f_123456789012".to_string(),
            filename: "test.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 1024,
            created_at: 1700000000,
            path: PathBuf::from("/tmp/test.txt"),
        };

        // Serialize
        let json = serde_json::to_string(&cached).unwrap();
        assert!(json.contains("f_123456789012"));
        assert!(json.contains("test.txt"));

        // Deserialize
        let deserialized: CachedFile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file_id, cached.file_id);
        assert_eq!(deserialized.filename, cached.filename);
        assert_eq!(deserialized.size_bytes, cached.size_bytes);
    }

    #[tokio::test]
    async fn test_file_cache_stats_max_bytes() {
        let temp_dir = std::env::temp_dir().join("test_stats_max_bytes");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let mut config = test_config(temp_dir.to_str().unwrap());
        config.max_cache_size_mb = 50;

        let cache = FileCache::new(config, "http://localhost:8080")
            .await
            .unwrap();

        let stats = cache.stats().await;
        assert_eq!(stats.max_bytes, 50 * 1024 * 1024);

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
