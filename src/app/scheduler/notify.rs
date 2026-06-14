use crate::core::{Result, Error};
use crate::config::Config;
use notify_rust::{Notification, Timeout};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use tracing::{info, warn, error, debug, trace};
use std::sync::Arc;

/// Represents the severity of a LiNix system event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    /// General system info (e.g., "Schedule update started").
    Info,
    /// Transaction success (e.g., "Daily upgrade completed").
    Success,
    /// Non-critical issue (e.g., "Snapshots reaching storage limit").
    Warning,
    /// Critical failure (e.g., "Transaction failed - WAL recovery required").
    Error,
}

impl NotificationLevel {
    /// Returns a UTF-8 icon for console and email headers.
    fn emoji(&self) -> &'static str {
        match self {
            Self::Info => "ℹ️",
            Self::Success => "✅",
            Self::Warning => "⚠️",
            Self::Error => "🚨",
        }
    }

    /// Returns the standardized OS title prefix.
    fn title_prefix(&self) -> &'static str {
        match self {
            Self::Info => "LiNix Info",
            Self::Success => "LiNix Success",
            Self::Warning => "LiNix Warning",
            Self::Error => "LiNix CRITICAL",
        }
    }
}

/// The orchestrator for LiNix system-wide alerts.
/// 
/// Modernized for 3.6.0: Provides full visual parity between Linux, 
/// macOS, and Windows using native OS notification protocols.
pub struct NotificationManager {
    config: Arc<Config>,
}

impl NotificationManager {
    /// Initializes the manager with shared kernel configuration.
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// High-level dispatch: Routes an alert to the requested channels.
    pub async fn notify(&self, channel: Option<&str>, level: NotificationLevel, subject: &str, body: &str) -> Result<()> {
        let channel = channel.unwrap_or("none");
        trace!("Notification: Level {:?} to channel '{}'", level, channel);

        match channel {
            "desktop" => {
                self.send_desktop(level, subject, body)?;
            }
            "email" => {
                self.send_email(level, subject, body).await?;
            }
            "all" => {
                let _ = self.send_desktop(level, subject, body);
                let _ = self.send_email(level, subject, body).await;
            }
            "none" => {
                info!("[{}] {}: {}", level.emoji(), subject, body);
            }
            _ => {
                warn!("Notification: Unknown channel '{}' requested.", channel);
            }
        }

        Ok(())
    }

    /// Sends a native OS desktop notification with platform-specific enhancements.
    fn send_desktop(&self, level: NotificationLevel, subject: &str, body: &str) -> Result<()> {
        let full_title = format!("{}: {}", level.title_prefix(), subject);
        
        let mut note = Notification::new();
        note.summary(&full_title)
            .body(body)
            .appname("LiNix")
            .timeout(Timeout::Milliseconds(8000)); // 8-second visibility

        // A+ Hardening: Platform-specific visual enhancements
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // Linux/X11/Wayland: Use standard FreeDesktop icon names
            let icon = match level {
                NotificationLevel::Error => "dialog-error",
                NotificationLevel::Warning => "dialog-warning",
                NotificationLevel::Success => "emblem-success",
                _ => "dialog-information",
            };
            note.icon(icon);
            note.hint(Hint::Urgency(match level {
                NotificationLevel::Error => notify_rust::Urgency::Critical,
                NotificationLevel::Warning => notify_rust::Urgency::Normal,
                _ => notify_rust::Urgency::Low,
            }));
        }

        #[cfg(target_os = "windows")]
        {
            // Windows: We use the Windows App ID if registered, or generic icons
            // notify-rust maps level to native Windows Toast types automatically
        }

        match note.show() {
            Ok(_) => debug!("Notification: Desktop alert dispatched."),
            Err(e) => {
                warn!("Notification: Desktop alerts unavailable ({}). Logging to console.", e);
                info!("[{}] {}: {}", level.emoji(), subject, body);
            }
        }

        Ok(())
    }

    /// Exhaustive SMTP Email implementation.
    /// Supports StartTLS and authenticated relays using pure Rust 'lettre'.
    async fn send_email(&self, level: NotificationLevel, subject: &str, body: &str) -> Result<()> {
        // 1. Logic: Verify if SMTP config is present
        let settings = match self.config.backend_settings.get("smtp") {
            Some(s) => s,
            None => {
                trace!("Notification: Email requested but [backend_settings.smtp] block missing.");
                return Ok(());
            }
        };

        let host = settings.get("host").ok_or_else(|| Error::Config("SMTP: host missing".into()))?;
        let port = settings.get("port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(587);
        let user = settings.get("user").ok_or_else(|| Error::Config("SMTP: user missing".into()))?;
        let pass = settings.get("pass").ok_or_else(|| Error::Config("SMTP: pass missing".into()))?;
        let to_addr = settings.get("to").ok_or_else(|| Error::Config("SMTP: to address missing".into()))?;

        // 2. Formatting: Professional transactional style
        let email_subject = format!("{} {} - {}", level.emoji(), level.title_prefix(), subject);
        let email_body = format!(
            "LiNix Mission-Critical Report\n\
             ==============================\n\n\
             Status:    {:?}\n\
             Timestamp: {}\n\
             Host:      {}\n\n\
             Message:\n\
             {}\n\n\
             ---\n\
             Automated Management via LiNix v3.6.0",
            level, chrono::Local::now().to_rfc2822(), crate::config::Config::get_hostname(), body
        );

        // 3. Build & Dispatch
        let email = Message::builder()
            .from(user.parse().map_err(|e| Error::Other(format!("SMTP From invalid: {}", e)))?)
            .to(to_addr.parse().map_err(|e| Error::Other(format!("SMTP To invalid: {}", e)))?)
            .subject(email_subject)
            .body(email_body)
            .map_err(|e| Error::Other(format!("Failed to build message: {}", e)))?;

        let creds = Credentials::new(user.to_string(), pass.to_string());
        
        // A+ Hardening: Use StartTLS for security on port 587
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| Error::Other(format!("SMTP Transport error: {}", e)))?
            .credentials(creds)
            .port(port)
            .build();

        match mailer.send(email).await {
            Ok(_) => info!("Notification: Report delivered to {}.", to_addr),
            Err(e) => error!("Notification: SMTP delivery FAILED via {}: {}", host, e),
        }

        Ok(())
    }
}