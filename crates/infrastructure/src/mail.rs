//! Outbound mail: an SMTP adapter and a logging stand-in for dev and for
//! deployments that have no mail server yet.

use std::time::Duration;

use async_trait::async_trait;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::AsyncSmtpTransportBuilder;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use application::ports::{Mail, Mailer};
use application::{AppError, AppResult};

/// Writes mail to the log instead of sending it. With `include_body` (local
/// development) the body, and so any token in it, is logged; without, only
/// recipient and subject, so production logs never carry tokens.
#[derive(Debug, Clone, Copy)]
pub struct LogMailer {
    pub include_body: bool,
}

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: &Mail) -> AppResult<()> {
        if self.include_body {
            tracing::info!(to = %mail.to, subject = %mail.subject, body = %mail.body, "mail (log transport)");
        } else {
            tracing::info!(to = %mail.to, subject = %mail.subject, "mail not delivered: no SMTP transport configured");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// Plain connection upgraded with STARTTLS (port 587).
    StartTls,
    /// TLS from the first byte (port 465).
    Implicit,
    /// No encryption; for a trusted local relay only.
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

const SEND_TIMEOUT: Duration = Duration::from_secs(15);

impl SmtpMailer {
    pub fn new(settings: &SmtpSettings) -> AppResult<Self> {
        let from = settings
            .from
            .parse::<Mailbox>()
            .map_err(|e| AppError::internal(format!("MAIL_FROM: {e}")))?;
        let builder: AsyncSmtpTransportBuilder = match settings.tls {
            SmtpTls::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
            }
            SmtpTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host),
            SmtpTls::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &settings.host,
            )),
        }
        .map_err(|e| AppError::internal(format!("SMTP_HOST: {e}")))?;
        let mut builder = builder.port(settings.port).timeout(Some(SEND_TIMEOUT));
        if let (Some(user), Some(pass)) = (&settings.username, &settings.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: &Mail) -> AppResult<()> {
        let to = mail
            .to
            .parse::<Mailbox>()
            .map_err(|e| AppError::Unavailable(format!("recipient: {e}")))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(mail.subject.clone())
            .body(mail.body.clone())
            .map_err(AppError::internal)?;
        self.transport
            .send(message)
            .await
            .map_err(|e| AppError::Unavailable(format!("smtp: {e}")))?;
        Ok(())
    }
}
