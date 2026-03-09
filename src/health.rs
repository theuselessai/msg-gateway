//! Health Check and Emergency Mode
//!
//! Monitors the target server (Pipelit) health and triggers emergency alerts
//! when it becomes unreachable.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::config::HealthCheckConfig;
use crate::message::InboundMessage;
use crate::server::AppState;

/// Health check state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Target server is healthy
    Healthy,
    /// Target server has some failures but not yet critical
    Degraded,
    /// Target server is down, emergency mode active
    Down,
    /// Target server is recovering from down state
    Recovering,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthState::Healthy => write!(f, "healthy"),
            HealthState::Degraded => write!(f, "degraded"),
            HealthState::Down => write!(f, "down"),
            HealthState::Recovering => write!(f, "recovering"),
        }
    }
}

/// Health monitor state
pub struct HealthMonitor {
    /// Current health state
    pub state: RwLock<HealthState>,
    /// Consecutive failure count
    pub failure_count: RwLock<u32>,
    /// Last successful health check
    pub last_healthy: RwLock<Option<Instant>>,
    /// Buffered messages during outage
    pub buffer: RwLock<VecDeque<InboundMessage>>,
    /// Maximum buffer size
    pub max_buffer_size: usize,
}

impl HealthMonitor {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            state: RwLock::new(HealthState::Healthy),
            failure_count: RwLock::new(0),
            last_healthy: RwLock::new(Some(Instant::now())),
            buffer: RwLock::new(VecDeque::new()),
            max_buffer_size,
        }
    }

    /// Get current state
    pub async fn get_state(&self) -> HealthState {
        *self.state.read().await
    }

    /// Record a successful health check
    pub async fn record_success(&self) -> Option<HealthState> {
        let old_state = *self.state.read().await;

        *self.failure_count.write().await = 0;
        *self.last_healthy.write().await = Some(Instant::now());

        let new_state = match old_state {
            HealthState::Down => HealthState::Recovering,
            HealthState::Recovering => HealthState::Healthy,
            _ => HealthState::Healthy,
        };

        if new_state != old_state {
            *self.state.write().await = new_state;
            tracing::info!(
                old_state = %old_state,
                new_state = %new_state,
                "Health state changed"
            );
            Some(new_state)
        } else {
            None
        }
    }

    /// Record a failed health check
    pub async fn record_failure(&self, alert_threshold: u32) -> Option<HealthState> {
        let old_state = *self.state.read().await;

        let mut count = self.failure_count.write().await;
        *count += 1;
        let failure_count = *count;
        drop(count);

        let new_state = if failure_count >= alert_threshold {
            HealthState::Down
        } else {
            HealthState::Degraded
        };

        if new_state != old_state {
            *self.state.write().await = new_state;
            tracing::warn!(
                old_state = %old_state,
                new_state = %new_state,
                failure_count = failure_count,
                "Health state changed"
            );
            Some(new_state)
        } else {
            None
        }
    }

    /// Buffer a message during outage
    pub async fn buffer_message(&self, message: InboundMessage) -> bool {
        let mut buffer = self.buffer.write().await;

        if buffer.len() >= self.max_buffer_size {
            // Drop oldest message
            buffer.pop_front();
            tracing::warn!("Message buffer full, dropping oldest message");
        }

        buffer.push_back(message);
        tracing::debug!(buffer_size = buffer.len(), "Message buffered");
        true
    }

    /// Drain buffered messages
    pub async fn drain_buffer(&self) -> Vec<InboundMessage> {
        let mut buffer = self.buffer.write().await;
        let messages: Vec<_> = buffer.drain(..).collect();
        tracing::info!(count = messages.len(), "Draining buffered messages");
        messages
    }

    /// Get buffer size
    pub async fn buffer_size(&self) -> usize {
        self.buffer.read().await.len()
    }

    /// Get last healthy timestamp
    pub async fn last_healthy_ago(&self) -> Option<Duration> {
        self.last_healthy.read().await.map(|t| t.elapsed())
    }
}

/// Start the health check loop
pub async fn start_health_check(
    state: Arc<AppState>,
    config_name: String,
    config: HealthCheckConfig,
) {
    let interval = Duration::from_secs(config.interval_seconds as u64);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    tracing::info!(
        name = %config_name,
        url = %config.url,
        interval_secs = config.interval_seconds,
        alert_after = config.alert_after_failures,
        "Starting health check"
    );

    loop {
        tokio::time::sleep(interval).await;

        let result = client.get(&config.url).send().await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                let state_change = state.health_monitor.record_success().await;

                if let Some(new_state) = state_change {
                    match new_state {
                        HealthState::Recovering => {
                            // Drain buffer and send messages
                            let messages = state.health_monitor.drain_buffer().await;
                            if !messages.is_empty() {
                                drain_buffered_messages(&state, messages).await;
                            }
                        }
                        HealthState::Healthy => {
                            // Send recovery notification
                            send_recovery_notification(&state, &config).await;
                        }
                        _ => {}
                    }
                }

                tracing::debug!(name = %config_name, "Health check passed");
            }
            Ok(resp) => {
                tracing::warn!(
                    name = %config_name,
                    status = %resp.status(),
                    "Health check failed (non-2xx)"
                );

                let state_change = state
                    .health_monitor
                    .record_failure(config.alert_after_failures)
                    .await;

                if let Some(HealthState::Down) = state_change {
                    send_emergency_alert(&state, &config).await;
                }
            }
            Err(e) => {
                tracing::warn!(
                    name = %config_name,
                    error = %e,
                    "Health check failed (network error)"
                );

                let state_change = state
                    .health_monitor
                    .record_failure(config.alert_after_failures)
                    .await;

                if let Some(HealthState::Down) = state_change {
                    send_emergency_alert(&state, &config).await;
                }
            }
        }
    }
}

/// Send emergency alert to all emergency credentials
async fn send_emergency_alert(state: &AppState, config: &HealthCheckConfig) {
    let app_config = state.config.read().await;

    let last_healthy = state
        .health_monitor
        .last_healthy_ago()
        .await
        .map(|d| format!("{:.0}s ago", d.as_secs_f64()))
        .unwrap_or_else(|| "unknown".to_string());

    let message = format!(
        "🚨 ALERT: Target server is unreachable!\n\
         Last healthy: {}\n\
         Messages are being buffered.",
        last_healthy
    );

    for cred_id in &config.notify_credentials {
        if let Some(cred) = app_config.credentials.get(cred_id)
            && cred.active
            && cred.emergency
        {
            tracing::info!(
                credential_id = %cred_id,
                "Sending emergency alert"
            );

            // For generic adapter, we can't really send an alert
            // since it requires a client to be connected.
            // For external adapters, we would POST to them.
            if cred.adapter == "generic" {
                tracing::warn!(
                    credential_id = %cred_id,
                    "Cannot send emergency alert via generic adapter (no persistent connection)"
                );
            } else {
                // TODO: POST to external adapter's /send endpoint
                tracing::info!(
                    credential_id = %cred_id,
                    adapter = %cred.adapter,
                    message = %message,
                    "Emergency alert (would be sent via adapter)"
                );
            }
        }
    }
}

/// Send recovery notification
async fn send_recovery_notification(state: &AppState, config: &HealthCheckConfig) {
    let app_config = state.config.read().await;

    let message = "✅ Target server has recovered. All systems operational.";

    for cred_id in &config.notify_credentials {
        if let Some(cred) = app_config.credentials.get(cred_id)
            && cred.active
            && cred.emergency
        {
            tracing::info!(
                credential_id = %cred_id,
                "Sending recovery notification"
            );

            if cred.adapter == "generic" {
                tracing::warn!(
                    credential_id = %cred_id,
                    "Cannot send recovery notification via generic adapter"
                );
            } else {
                // TODO: POST to external adapter's /send endpoint
                tracing::info!(
                    credential_id = %cred_id,
                    adapter = %cred.adapter,
                    message = %message,
                    "Recovery notification (would be sent via adapter)"
                );
            }
        }
    }
}

/// Drain buffered messages to the target server
async fn drain_buffered_messages(state: &AppState, messages: Vec<InboundMessage>) {
    use crate::backend::{create_adapter, resolve_target};

    // Hoist invariants outside the loop
    let config = state.config.read().await;
    let gateway_ctx = crate::backend::GatewayContext {
        gateway_url: format!("http://{}", config.gateway.listen),
        send_token: config.auth.send_token.clone(),
    };
    let default_target = config.gateway.default_target.clone();
    drop(config);

    for message in messages {
        // Per-message: only acquire lock for credential lookup
        let config = state.config.read().await;
        let adapter = if let Some(credential) = config.credentials.get(&message.credential_id) {
            let target = resolve_target(credential, &default_target);
            match create_adapter(target, Some(&gateway_ctx), credential.config.as_ref()) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        message_id = %message.source.message_id,
                        credential_id = %message.credential_id,
                        error = %e,
                        "Failed to create backend adapter for buffered message"
                    );
                    continue;
                }
            }
        } else {
            // Credential no longer exists, use default target
            match create_adapter(&default_target, Some(&gateway_ctx), None) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        message_id = %message.source.message_id,
                        error = %e,
                        "Failed to create default backend adapter"
                    );
                    continue;
                }
            }
        };
        drop(config);

        match adapter.send_message(&message).await {
            Ok(()) => {
                tracing::debug!(
                    message_id = %message.source.message_id,
                    "Buffered message delivered"
                );
            }
            Err(e) => {
                tracing::error!(
                    message_id = %message.source.message_id,
                    error = %e,
                    "Failed to deliver buffered message"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_state_display() {
        assert_eq!(format!("{}", HealthState::Healthy), "healthy");
        assert_eq!(format!("{}", HealthState::Degraded), "degraded");
        assert_eq!(format!("{}", HealthState::Down), "down");
        assert_eq!(format!("{}", HealthState::Recovering), "recovering");
    }

    #[tokio::test]
    async fn test_health_monitor_new() {
        let monitor = HealthMonitor::new(100);
        assert_eq!(monitor.get_state().await, HealthState::Healthy);
        assert_eq!(monitor.buffer_size().await, 0);
        assert_eq!(monitor.max_buffer_size, 100);
    }

    #[tokio::test]
    async fn test_health_monitor_record_success() {
        let monitor = HealthMonitor::new(100);

        // Initial state is healthy
        assert_eq!(monitor.get_state().await, HealthState::Healthy);

        // Recording success should keep it healthy
        let state_change = monitor.record_success().await;
        assert!(state_change.is_none()); // No state change
        assert_eq!(monitor.get_state().await, HealthState::Healthy);
    }

    #[tokio::test]
    async fn test_health_monitor_record_failure() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 3;

        // First failure - Healthy -> Degraded
        let change1 = monitor.record_failure(alert_threshold).await;
        assert_eq!(change1, Some(HealthState::Degraded));
        assert_eq!(monitor.get_state().await, HealthState::Degraded);

        // Second failure - stays Degraded (no state change)
        let change2 = monitor.record_failure(alert_threshold).await;
        assert!(change2.is_none());
        assert_eq!(monitor.get_state().await, HealthState::Degraded);

        // Third failure - Degraded -> Down (threshold reached)
        let change3 = monitor.record_failure(alert_threshold).await;
        assert_eq!(change3, Some(HealthState::Down));
        assert_eq!(monitor.get_state().await, HealthState::Down);
    }

    #[tokio::test]
    async fn test_health_monitor_recovery() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 1;

        // Go to Down state
        monitor.record_failure(alert_threshold).await;
        assert_eq!(monitor.get_state().await, HealthState::Down);

        // First success - should go to Recovering
        let change1 = monitor.record_success().await;
        assert_eq!(change1, Some(HealthState::Recovering));
        assert_eq!(monitor.get_state().await, HealthState::Recovering);

        // Second success - should go to Healthy
        let change2 = monitor.record_success().await;
        assert_eq!(change2, Some(HealthState::Healthy));
        assert_eq!(monitor.get_state().await, HealthState::Healthy);
    }

    #[tokio::test]
    async fn test_health_monitor_buffer() {
        use crate::message::{InboundMessage, MessageSource, UserInfo};
        use chrono::Utc;

        let monitor = HealthMonitor::new(2); // Small buffer for testing

        let make_message = |id: &str| InboundMessage {
            route: serde_json::json!("test"),
            credential_id: "cred1".to_string(),
            source: MessageSource {
                protocol: "test".to_string(),
                chat_id: "chat1".to_string(),
                message_id: id.to_string(),
                reply_to_message_id: None,
                from: UserInfo {
                    id: "user1".to_string(),
                    username: None,
                    display_name: None,
                },
            },
            text: "test message".to_string(),
            attachments: vec![],
            timestamp: Utc::now(),
            extra_data: None,
        };

        // Buffer first message
        assert!(monitor.buffer_message(make_message("msg1")).await);
        assert_eq!(monitor.buffer_size().await, 1);

        // Buffer second message
        assert!(monitor.buffer_message(make_message("msg2")).await);
        assert_eq!(monitor.buffer_size().await, 2);

        // Buffer third message - should drop oldest
        assert!(monitor.buffer_message(make_message("msg3")).await);
        assert_eq!(monitor.buffer_size().await, 2);

        // Drain buffer
        let messages = monitor.drain_buffer().await;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].source.message_id, "msg2"); // First was dropped
        assert_eq!(messages[1].source.message_id, "msg3");

        // Buffer should be empty now
        assert_eq!(monitor.buffer_size().await, 0);
    }

    #[tokio::test]
    async fn test_last_healthy_ago() {
        let monitor = HealthMonitor::new(100);

        // Initially should have a last_healthy timestamp
        let duration = monitor.last_healthy_ago().await;
        assert!(duration.is_some());
        // Should be very recent (less than 1 second)
        assert!(duration.unwrap().as_secs() < 1);

        // After recording success, should update timestamp
        tokio::time::sleep(Duration::from_millis(10)).await;
        monitor.record_success().await;
        let duration2 = monitor.last_healthy_ago().await;
        assert!(duration2.is_some());
        // Use lenient threshold to avoid flaky tests on slow/busy systems
        assert!(duration2.unwrap().as_millis() < 500);
    }

    #[tokio::test]
    async fn test_health_state_equality() {
        // Test PartialEq and Eq implementations
        assert_eq!(HealthState::Healthy, HealthState::Healthy);
        assert_eq!(HealthState::Degraded, HealthState::Degraded);
        assert_eq!(HealthState::Down, HealthState::Down);
        assert_eq!(HealthState::Recovering, HealthState::Recovering);

        assert_ne!(HealthState::Healthy, HealthState::Degraded);
        assert_ne!(HealthState::Down, HealthState::Recovering);
    }

    #[tokio::test]
    #[allow(clippy::clone_on_copy)]
    async fn test_health_state_clone_copy() {
        // Test Clone and Copy implementations
        let state = HealthState::Healthy;
        let cloned = state.clone(); // Intentionally testing clone on Copy type
        let copied = state;

        assert_eq!(state, cloned);
        assert_eq!(state, copied);
    }

    #[tokio::test]
    async fn test_multiple_failures_staying_down() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 2;

        // Go to Down state
        monitor.record_failure(alert_threshold).await;
        monitor.record_failure(alert_threshold).await;
        assert_eq!(monitor.get_state().await, HealthState::Down);

        // Additional failures should not cause state changes
        let change = monitor.record_failure(alert_threshold).await;
        assert!(change.is_none());
        assert_eq!(monitor.get_state().await, HealthState::Down);

        let change = monitor.record_failure(alert_threshold).await;
        assert!(change.is_none());
        assert_eq!(monitor.get_state().await, HealthState::Down);
    }

    #[tokio::test]
    async fn test_failure_count_reset_on_success() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 3;

        // Add some failures (but not enough to go Down)
        monitor.record_failure(alert_threshold).await;
        monitor.record_failure(alert_threshold).await;
        assert_eq!(monitor.get_state().await, HealthState::Degraded);

        // Success should reset failure count
        monitor.record_success().await;
        assert_eq!(monitor.get_state().await, HealthState::Healthy);

        // Now failures should start from 0 again
        let change = monitor.record_failure(alert_threshold).await;
        assert_eq!(change, Some(HealthState::Degraded));

        // Need 2 more failures to reach threshold, not just 1
        let change = monitor.record_failure(alert_threshold).await;
        assert!(change.is_none()); // Still degraded

        let change = monitor.record_failure(alert_threshold).await;
        assert_eq!(change, Some(HealthState::Down));
    }

    #[tokio::test]
    async fn test_drain_empty_buffer() {
        let monitor = HealthMonitor::new(100);

        // Draining empty buffer should return empty vec
        let messages = monitor.drain_buffer().await;
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn test_buffer_exactly_at_max_size() {
        use crate::message::{InboundMessage, MessageSource, UserInfo};
        use chrono::Utc;

        let monitor = HealthMonitor::new(3);

        let make_message = |id: &str| InboundMessage {
            route: serde_json::json!("test"),
            credential_id: "cred1".to_string(),
            source: MessageSource {
                protocol: "test".to_string(),
                chat_id: "chat1".to_string(),
                message_id: id.to_string(),
                reply_to_message_id: None,
                from: UserInfo {
                    id: "user1".to_string(),
                    username: None,
                    display_name: None,
                },
            },
            text: "test message".to_string(),
            attachments: vec![],
            timestamp: Utc::now(),
            extra_data: None,
        };

        // Fill buffer exactly to capacity
        monitor.buffer_message(make_message("msg1")).await;
        monitor.buffer_message(make_message("msg2")).await;
        monitor.buffer_message(make_message("msg3")).await;
        assert_eq!(monitor.buffer_size().await, 3);

        // Next message should cause oldest to be dropped
        monitor.buffer_message(make_message("msg4")).await;
        assert_eq!(monitor.buffer_size().await, 3);

        let messages = monitor.drain_buffer().await;
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].source.message_id, "msg2");
        assert_eq!(messages[1].source.message_id, "msg3");
        assert_eq!(messages[2].source.message_id, "msg4");
    }

    #[tokio::test]
    async fn test_recovery_from_degraded_state() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 5;

        // Go to Degraded state (but not Down)
        monitor.record_failure(alert_threshold).await;
        assert_eq!(monitor.get_state().await, HealthState::Degraded);

        // Success from Degraded should go directly to Healthy
        let change = monitor.record_success().await;
        assert_eq!(change, Some(HealthState::Healthy));
        assert_eq!(monitor.get_state().await, HealthState::Healthy);
    }

    #[tokio::test]
    async fn test_health_state_debug() {
        // Test Debug implementation
        let state = HealthState::Healthy;
        let debug_str = format!("{:?}", state);
        assert_eq!(debug_str, "Healthy");

        let state = HealthState::Down;
        let debug_str = format!("{:?}", state);
        assert_eq!(debug_str, "Down");
    }

    #[tokio::test]
    async fn test_threshold_of_one() {
        let monitor = HealthMonitor::new(100);
        let alert_threshold = 1;

        // First failure should immediately go to Down
        let change = monitor.record_failure(alert_threshold).await;
        assert_eq!(change, Some(HealthState::Down));
        assert_eq!(monitor.get_state().await, HealthState::Down);
    }

    #[tokio::test]
    async fn test_concurrent_buffer_access() {
        use crate::message::{InboundMessage, MessageSource, UserInfo};
        use chrono::Utc;
        use std::sync::Arc;

        let monitor = Arc::new(HealthMonitor::new(100));

        let make_message = |id: &str| InboundMessage {
            route: serde_json::json!("test"),
            credential_id: "cred1".to_string(),
            source: MessageSource {
                protocol: "test".to_string(),
                chat_id: "chat1".to_string(),
                message_id: id.to_string(),
                reply_to_message_id: None,
                from: UserInfo {
                    id: "user1".to_string(),
                    username: None,
                    display_name: None,
                },
            },
            text: "test message".to_string(),
            attachments: vec![],
            timestamp: Utc::now(),
            extra_data: None,
        };

        // Spawn multiple tasks to buffer messages concurrently
        let mut handles = vec![];
        for i in 0..10 {
            let monitor_clone = Arc::clone(&monitor);
            let msg = make_message(&format!("msg{}", i));
            handles.push(tokio::spawn(async move {
                monitor_clone.buffer_message(msg).await
            }));
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // All messages should be buffered
        assert_eq!(monitor.buffer_size().await, 10);
    }
}
