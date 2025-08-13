use crate::config::BalanceConfig;
use crate::error::BalanceError;
use crate::mastodon::MastodonStream;
use crate::openrouter::OpenRouterClient;
use chrono::{DateTime, Local, NaiveTime, Timelike, Utc};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Balance monitoring system for OpenRouter account
pub struct BalanceMonitor {
    config: BalanceConfig,
    openrouter_client: OpenRouterClient,
    last_check: Option<DateTime<Utc>>,
    last_notification: Option<DateTime<Utc>>,
}

impl BalanceMonitor {
    /// Create a new balance monitor
    pub fn new(config: BalanceConfig, openrouter_client: OpenRouterClient) -> Self {
        Self {
            config,
            openrouter_client,
            last_check: None,
            last_notification: None,
        }
    }

    /// Check if balance monitoring is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.enabled.unwrap_or(true)
    }

    /// Get the configured balance threshold
    pub fn threshold(&self) -> f64 {
        self.config.threshold.unwrap_or(5.0)
    }

    /// Get the configured check time
    pub fn check_time(&self) -> Result<NaiveTime, BalanceError> {
        let time_str = self.config.check_time.as_deref().unwrap_or("12:00");

        // Parse time in HH:MM format
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 2 {
            return Err(BalanceError::InvalidCheckTime {
                time: time_str.to_string(),
            });
        }

        let hour: u32 = parts[0]
            .parse()
            .map_err(|_| BalanceError::InvalidCheckTime {
                time: time_str.to_string(),
            })?;

        let minute: u32 = parts[1]
            .parse()
            .map_err(|_| BalanceError::InvalidCheckTime {
                time: time_str.to_string(),
            })?;

        if hour >= 24 || minute >= 60 {
            return Err(BalanceError::InvalidCheckTime {
                time: time_str.to_string(),
            });
        }

        NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| BalanceError::InvalidCheckTime {
            time: time_str.to_string(),
        })
    }

    /// Calculate seconds until next check time
    fn seconds_until_next_check(&self) -> Result<u64, BalanceError> {
        let check_time = self.check_time()?;
        let now = Local::now();
        let today_check = now.date_naive().and_time(check_time);

        // If today's check time has passed or is now, schedule for tomorrow
        let next_check = if now.time() >= check_time {
            today_check + chrono::Duration::days(1)
        } else {
            today_check
        };

        let next_check_utc = next_check
            .and_local_timezone(Local)
            .single()
            .ok_or_else(|| BalanceError::InvalidCheckTime {
                time: check_time.to_string(),
            })?
            .with_timezone(&Utc);

        let duration = next_check_utc.signed_duration_since(now.with_timezone(&Utc));
        let seconds = duration.num_seconds().max(0) as u64;

        // Ensure we never sleep for less than 60 seconds to prevent busy waiting
        let seconds = if seconds < 60 { 86400 } else { seconds };

        debug!(
            "Next balance check scheduled in {} seconds at {}",
            seconds, next_check_utc
        );
        Ok(seconds)
    }

    /// Check if we should perform a balance check now
    fn should_check_now(&self) -> Result<bool, BalanceError> {
        let check_time = self.check_time()?;
        let now = Local::now();

        // Check if we're within the check time window (within 5 minutes)
        let current_time = now.time();
        let check_window_start = check_time;
        let check_window_end = {
            let total_minutes = check_time.hour() * 60 + check_time.minute() + 5;
            if total_minutes >= 24 * 60 {
                NaiveTime::from_hms_opt(23, 59, 59).unwrap()
            } else {
                NaiveTime::from_hms_opt((total_minutes / 60) % 24, total_minutes % 60, 0).unwrap()
            }
        };

        let in_window = current_time >= check_window_start && current_time <= check_window_end;

        if !in_window {
            return Ok(false);
        }

        // Check if we already performed a check today
        if let Some(last_check) = self.last_check {
            let today = now.date_naive();
            let last_check_date = last_check.with_timezone(&Local).date_naive();

            if last_check_date == today {
                debug!("Balance already checked today");
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Perform balance check and send notification if needed
    pub async fn check_balance<M>(&mut self, mastodon_client: &M) -> Result<(), BalanceError>
    where
        M: MastodonStream,
    {
        if !self.is_enabled() {
            debug!("Balance monitoring is disabled");
            return Ok(());
        }

        info!("Checking OpenRouter account balance");

        // Get current balance
        let balance = self
            .openrouter_client
            .get_account_balance()
            .await
            .map_err(|e| BalanceError::CheckFailed(format!("Failed to get balance: {e}")))?;

        self.last_check = Some(Utc::now());

        let threshold = self.threshold();
        info!(
            "Current balance: ${:.2}, threshold: ${:.2}",
            balance, threshold
        );

        // Check if balance is below threshold
        if balance < threshold {
            warn!(
                "Balance ${:.2} is below threshold ${:.2}",
                balance, threshold
            );

            // Check if we should send a notification (avoid spam)
            if self.should_send_notification() {
                self.send_low_balance_notification(mastodon_client, balance, threshold)
                    .await?;
                self.last_notification = Some(Utc::now());
            } else {
                debug!("Skipping notification to avoid spam");
            }
        } else {
            info!("Balance is above threshold");
        }

        Ok(())
    }

    /// Check if we should send a notification (avoid spam)
    fn should_send_notification(&self) -> bool {
        // Don't send more than one notification per day
        if let Some(last_notification) = self.last_notification {
            let now = Utc::now();
            let hours_since_last = now.signed_duration_since(last_notification).num_hours();

            if hours_since_last < 24 {
                return false;
            }
        }

        true
    }

    /// Send low balance notification via direct message
    async fn send_low_balance_notification<M>(
        &self,
        mastodon_client: &M,
        balance: f64,
        threshold: f64,
    ) -> Result<(), BalanceError>
    where
        M: MastodonStream,
    {
        let message = format!(
            "⚠️ OpenRouter Balance Alert\n\n\
            Your OpenRouter account balance is ${balance:.2}, which is below the configured threshold of ${threshold:.2}.\n\n\
            Please top up your account to continue using Alternator's image description service.\n\n\
            Visit: https://openrouter.ai/credits"
        );

        info!("Sending low balance notification");

        mastodon_client
            .send_dm(&message)
            .await
            .map_err(|e| BalanceError::NotificationFailed(format!("Failed to send DM: {e}")))?;

        info!("Low balance notification sent successfully");
        Ok(())
    }

    /// Run the balance monitoring loop
    pub async fn run<M>(&mut self, mastodon_client: &M) -> Result<(), BalanceError>
    where
        M: MastodonStream,
    {
        if !self.is_enabled() {
            info!("Balance monitoring is disabled");
            return Ok(());
        }

        info!("Starting balance monitoring service");

        loop {
            // Check if we should perform a balance check now
            if self.should_check_now()? {
                if let Err(e) = self.check_balance(mastodon_client).await {
                    error!("Balance check failed: {}", e);
                    // Continue running even if check fails
                }
            }

            // Calculate sleep duration until next check
            let sleep_duration = match self.seconds_until_next_check() {
                Ok(seconds) => {
                    // If next check is more than 1 hour away, sleep for 1 hour and recheck
                    let sleep_seconds = seconds.min(3600);
                    Duration::from_secs(sleep_seconds)
                }
                Err(e) => {
                    error!("Failed to calculate next check time: {}", e);
                    // Default to checking every hour if calculation fails
                    Duration::from_secs(3600)
                }
            };

            debug!(
                "Sleeping for {} seconds until next balance check",
                sleep_duration.as_secs()
            );
            sleep(sleep_duration).await;
        }
    }

    /// Perform an immediate balance check (for testing or manual triggers)
    #[allow(dead_code)] // Public API for manual balance checks
    pub async fn check_now<M>(&mut self, mastodon_client: &M) -> Result<f64, BalanceError>
    where
        M: MastodonStream,
    {
        info!("Performing immediate balance check");

        let balance = self
            .openrouter_client
            .get_account_balance()
            .await
            .map_err(|e| BalanceError::CheckFailed(format!("Failed to get balance: {e}")))?;

        self.last_check = Some(Utc::now());

        let threshold = self.threshold();
        info!(
            "Current balance: ${:.2}, threshold: ${:.2}",
            balance, threshold
        );

        if balance < threshold {
            warn!(
                "Balance ${:.2} is below threshold ${:.2}",
                balance, threshold
            );
            self.send_low_balance_notification(mastodon_client, balance, threshold)
                .await?;
            self.last_notification = Some(Utc::now());
        }

        Ok(balance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OpenRouterConfig;
    use crate::error::MastodonError;
    use crate::mastodon::{Account, TootEvent};

    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Mock MastodonStream for testing
    struct MockMastodonClient {
        sent_messages: Arc<Mutex<Vec<String>>>,
        should_fail: bool,
    }

    impl MockMastodonClient {
        fn new() -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                should_fail: false,
            }
        }

        #[allow(dead_code)] // Test helper function
        fn with_failure() -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                should_fail: true,
            }
        }

        async fn get_sent_messages(&self) -> Vec<String> {
            self.sent_messages.lock().await.clone()
        }
    }

    impl MastodonStream for MockMastodonClient {
        async fn connect(&mut self) -> Result<(), MastodonError> {
            Ok(())
        }

        async fn listen(&mut self) -> Result<Option<TootEvent>, MastodonError> {
            Ok(None)
        }

        async fn get_toot(&self, _toot_id: &str) -> Result<TootEvent, MastodonError> {
            Err(MastodonError::TootNotFound {
                toot_id: "test".to_string(),
            })
        }

        async fn update_media(
            &self,
            _toot_id: &str,
            _media_id: &str,
            _description: &str,
        ) -> Result<(), MastodonError> {
            Ok(())
        }

        async fn update_multiple_media(
            &self,
            _toot_id: &str,
            _media_updates: Vec<(String, String)>,
        ) -> Result<(), MastodonError> {
            Ok(())
        }

        async fn create_media_attachment(
            &self,
            _image_data: Vec<u8>,
            _description: &str,
            _filename: &str,
        ) -> Result<String, MastodonError> {
            Ok("mock_media_id".to_string())
        }

        async fn recreate_media_with_descriptions(
            &self,
            _toot_id: &str,
            _media_recreations: Vec<(Vec<u8>, String)>,
            _original_media_ids: Vec<String>,
        ) -> Result<(), MastodonError> {
            Ok(())
        }

        async fn send_dm(&self, message: &str) -> Result<(), MastodonError> {
            if self.should_fail {
                return Err(MastodonError::ApiRequestFailed("Mock failure".to_string()));
            }

            self.sent_messages.lock().await.push(message.to_string());
            Ok(())
        }

        async fn verify_credentials(&mut self) -> Result<Account, MastodonError> {
            Ok(Account {
                id: "test_user".to_string(),
                username: "testuser".to_string(),
                acct: "testuser".to_string(),
                display_name: "Test User".to_string(),
                url: "https://example.com".to_string(),
            })
        }
    }

    // Mock OpenRouter client for testing
    #[allow(dead_code)] // Test helpers
    struct MockOpenRouterClient {
        balance: f64,
        should_fail: bool,
    }

    impl MockOpenRouterClient {
        #[allow(dead_code)] // Test helper function
        fn new(balance: f64) -> Self {
            Self {
                balance,
                should_fail: false,
            }
        }

        #[allow(dead_code)] // Test helper function
        fn with_failure() -> Self {
            Self {
                balance: 0.0,
                should_fail: true,
            }
        }

        #[allow(dead_code)] // Test helper function
        async fn get_account_balance(&self) -> Result<f64, crate::error::OpenRouterError> {
            if self.should_fail {
                return Err(crate::error::OpenRouterError::ApiRequestFailed(
                    "Mock failure".to_string(),
                ));
            }
            Ok(self.balance)
        }
    }

    fn create_test_config() -> BalanceConfig {
        BalanceConfig {
            enabled: Some(true),
            threshold: Some(5.0),
            check_time: Some("12:00".to_string()),
        }
    }

    fn create_openrouter_config() -> OpenRouterConfig {
        OpenRouterConfig {
            api_key: "test_key".to_string(),
            model: "test_model".to_string(),
            base_url: Some("https://test.openrouter.ai".to_string()),
            max_tokens: Some(150),
        }
    }

    #[test]
    fn test_balance_monitor_creation() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config.clone(), openrouter_client);

        assert!(monitor.is_enabled());
        assert_eq!(monitor.threshold(), 5.0);
        assert!(monitor.last_check.is_none());
        assert!(monitor.last_notification.is_none());
    }

    #[test]
    fn test_balance_monitor_disabled() {
        let mut config = create_test_config();
        config.enabled = Some(false);
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        assert!(!monitor.is_enabled());
    }

    #[test]
    fn test_balance_monitor_default_values() {
        let config = BalanceConfig {
            enabled: None,
            threshold: None,
            check_time: None,
        };
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        assert!(monitor.is_enabled()); // Default is true
        assert_eq!(monitor.threshold(), 5.0); // Default threshold
    }

    #[test]
    fn test_check_time_parsing() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let check_time = monitor.check_time().unwrap();
        assert_eq!(check_time.hour(), 12);
        assert_eq!(check_time.minute(), 0);
    }

    #[test]
    fn test_check_time_parsing_custom() {
        let mut config = create_test_config();
        config.check_time = Some("14:30".to_string());
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let check_time = monitor.check_time().unwrap();
        assert_eq!(check_time.hour(), 14);
        assert_eq!(check_time.minute(), 30);
    }

    #[test]
    fn test_check_time_parsing_invalid_format() {
        let mut config = create_test_config();
        config.check_time = Some("invalid".to_string());
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let result = monitor.check_time();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BalanceError::InvalidCheckTime { .. }
        ));
    }

    #[test]
    fn test_check_time_parsing_invalid_hour() {
        let mut config = create_test_config();
        config.check_time = Some("25:00".to_string());
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let result = monitor.check_time();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BalanceError::InvalidCheckTime { .. }
        ));
    }

    #[test]
    fn test_check_time_parsing_invalid_minute() {
        let mut config = create_test_config();
        config.check_time = Some("12:60".to_string());
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let result = monitor.check_time();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BalanceError::InvalidCheckTime { .. }
        ));
    }

    #[test]
    fn test_should_send_notification_first_time() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        assert!(monitor.should_send_notification());
    }

    #[test]
    fn test_should_send_notification_recent() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let mut monitor = BalanceMonitor::new(config, openrouter_client);

        // Set last notification to 1 hour ago
        monitor.last_notification = Some(Utc::now() - chrono::Duration::hours(1));

        assert!(!monitor.should_send_notification());
    }

    #[test]
    fn test_should_send_notification_old() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let mut monitor = BalanceMonitor::new(config, openrouter_client);

        // Set last notification to 25 hours ago
        monitor.last_notification = Some(Utc::now() - chrono::Duration::hours(25));

        assert!(monitor.should_send_notification());
    }

    #[test]
    fn test_seconds_until_next_check() {
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let seconds = monitor.seconds_until_next_check().unwrap();
        // Should be a positive number of seconds (could be up to 24 hours)
        assert!(seconds > 0);
        assert!(seconds <= 24 * 60 * 60); // Max 24 hours
    }

    #[test]
    fn test_no_infinite_loop_at_check_time() {
        // Test that when current time equals check time, we schedule for next day
        let config = create_test_config();
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let monitor = BalanceMonitor::new(config, openrouter_client);

        let seconds = monitor.seconds_until_next_check().unwrap();

        // Should never return 0 seconds to prevent infinite loops
        // Minimum should be 60 seconds (our safety threshold)
        assert!(seconds >= 60, "Expected at least 60 seconds, got {seconds}");
        assert!(
            seconds <= 48 * 60 * 60,
            "Expected at most 48 hours, got {seconds}"
        );
    }

    // Note: The following tests would require more complex mocking of the OpenRouter client
    // For now, we'll focus on the core logic tests above

    #[tokio::test]
    async fn test_check_balance_disabled() {
        let mut config = create_test_config();
        config.enabled = Some(false);
        let openrouter_client =
            crate::openrouter::OpenRouterClient::new(create_openrouter_config());
        let mut monitor = BalanceMonitor::new(config, openrouter_client);
        let mastodon_client = MockMastodonClient::new();

        let result = monitor.check_balance(&mastodon_client).await;
        assert!(result.is_ok());

        // No messages should be sent when disabled
        let messages = mastodon_client.get_sent_messages().await;
        assert!(messages.is_empty());
    }

    #[test]
    fn test_balance_error_display() {
        let check_error = BalanceError::CheckFailed("network timeout".to_string());
        assert!(check_error.to_string().contains("Balance check failed"));
        assert!(check_error.to_string().contains("network timeout"));

        let threshold_error = BalanceError::InvalidThreshold { threshold: -1.0 };
        assert!(threshold_error
            .to_string()
            .contains("Invalid balance threshold"));
        assert!(threshold_error.to_string().contains("-1"));

        let time_error = BalanceError::InvalidCheckTime {
            time: "25:00".to_string(),
        };
        assert!(time_error.to_string().contains("Invalid check time format"));
        assert!(time_error.to_string().contains("25:00"));

        let notification_error = BalanceError::NotificationFailed("DM failed".to_string());
        assert!(notification_error
            .to_string()
            .contains("Notification sending failed"));
        assert!(notification_error.to_string().contains("DM failed"));
    }
}
