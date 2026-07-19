// src/app/scheduler/notify.rs

use crate::config::Config;
use crate::core::{Error, Result};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
#[allow(unused_imports)]
use notify_rust::{Hint, Notification, Timeout};
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationLevel {
    fn title_prefix(&self) -> &'static str {
        match self {
            Self::Info => "LiNix Info",
            Self::Success => "LiNix Success",
            Self::Warning => "LiNix Warning",
            Self::Error => "LiNix Error",
        }
    }
}

pub struct NotificationManager {
    config: Arc<Config>,
}

impl NotificationManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub async fn notify(
        &self,
        channel: Option<&str>,
        level: NotificationLevel,
        subject: &str,
        body: &str,
    ) -> Result<()> {
        let channel = channel.unwrap_or("none");
        trace!("Level {:?} to channel '{}'", level, channel);

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
                info!("[{}] {}: {}", level.title_prefix(), subject, body);
            }
            _ => {
                warn!("Unknown channel '{}' requested.", channel);
            }
        }

        Ok(())
    }

    fn send_desktop(&self, level: NotificationLevel, subject: &str, body: &str) -> Result<()> {
        let full_title = format!("{}: {}", level.title_prefix(), subject);

        let mut note = Notification::new();
        note.summary(&full_title)
            .body(body)
            .appname("LiNix")
            .timeout(Timeout::Milliseconds(8000));

        #[cfg(all(unix, not(target_os = "macos")))]
        {
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
            // Windows: notify-rust handles it automatically.
        }

        match note.show() {
            Ok(_) => debug!("Desktop alert dispatched."),
            Err(e) => {
                warn!(
                    "Desktop alerts unavailable ({}). Logging to console.",
                    e
                );
                info!("[{}] {}: {}", level.title_prefix(), subject, body);
            }
        }

        Ok(())
    }

    async fn send_email(&self, level: NotificationLevel, subject: &str, body: &str) -> Result<()> {
        let settings = match self.config.backend_settings.get("smtp") {
            Some(s) => s,
            None => {
                trace!("Email requested but [backend_settings.smtp] block missing.");
                return Ok(());
            }
        };

        let host = settings
            .get("host")
            .ok_or_else(|| Error::Config("SMTP: host missing".into()))?;
        let port = settings
            .get("port")
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(587);
        let user = settings
            .get("user")
            .ok_or_else(|| Error::Config("SMTP: user missing".into()))?;
        let pass = settings
            .get("pass")
            .ok_or_else(|| Error::Config("SMTP: pass missing".into()))?;
        let to_addr = settings
            .get("to")
            .ok_or_else(|| Error::Config("SMTP: to address missing".into()))?;

        let email_subject = format!("{} - {}", level.title_prefix(), subject);
        let email_body = format!(
            "LiNix Report\n\
             ============\n\n\
             Status:    {:?}\n\
             Timestamp: {}\n\
             Host:      {}\n\n\
             Message:\n\
             {}\n\n\
             ---\n\
             Automated Management via LiNix",
            level,
            chrono::Local::now().to_rfc2822(),
            crate::config::Config::get_hostname(),
            body
        );

        let email = Message::builder()
            .from(
                user.parse()
                    .map_err(|e| Error::Other(format!("SMTP From invalid: {}", e)))?,
            )
            .to(to_addr
                .parse()
                .map_err(|e| Error::Other(format!("SMTP To invalid: {}", e)))?)
            .subject(email_subject)
            .body(email_body)
            .map_err(|e| Error::Other(format!("Failed to build message: {}", e)))?;

        let creds = Credentials::new(user.to_string(), pass.to_string());

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| Error::Other(format!("SMTP Transport error: {}", e)))?
            .credentials(creds)
            .port(port)
            .build();

        match mailer.send(email).await {
            Ok(_) => info!("Report delivered to {}.", to_addr),
            Err(e) => error!("SMTP delivery FAILED via {}: {}", host, e),
        }

        Ok(())
    }
}
