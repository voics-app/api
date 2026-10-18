//! HTTP client for the `messages` service's internal API (P4). Used by api
//! for bot, webhook and interaction messages, which the messages service owns.

use async_trait::async_trait;
use reqwest::{Client, Method, Url};
use serde_json::Value;

use application::ports::{MessageService, StoredMessage};
use application::{AppError, AppResult};
use domain::Snowflake;

const TOKEN_HEADER: &str = "X-Internal-Token";

pub struct HttpMessages {
    http: Client,
    base: Url,
    internal_token: String,
}

impl HttpMessages {
    pub fn new(base: Url, internal_token: String) -> Self {
        Self {
            http: Client::new(),
            base,
            internal_token,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> AppResult<Value> {
        let url = self.base.join(path).map_err(AppError::internal)?;
        let mut request = self
            .http
            .request(method, url)
            .header(TOKEN_HEADER, &self.internal_token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AppError::Unavailable(format!("messages: {e}")))?;
        let status = response.status();
        if status.is_server_error() {
            return Err(AppError::Unavailable(format!(
                "messages responded {status}"
            )));
        }
        if !status.is_success() {
            return Err(AppError::BadRequest {
                code: 0,
                message: format!("messages responded {status}"),
            });
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        response
            .json()
            .await
            .map_err(|e| AppError::Unavailable(format!("messages body: {e}")))
    }
}

fn stored(body: Value) -> AppResult<StoredMessage> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<Snowflake>().ok())
        .ok_or_else(|| AppError::Unavailable("messages returned no message id".into()))?;
    Ok(StoredMessage { id, body })
}

#[async_trait]
impl MessageService for HttpMessages {
    async fn create(&self, channel_id: Snowflake, message: Value) -> AppResult<StoredMessage> {
        let path = format!("internal/v1/channels/{channel_id}/messages");
        stored(self.send(Method::POST, &path, Some(message)).await?)
    }

    async fn edit(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        patch: Value,
    ) -> AppResult<StoredMessage> {
        let path = format!("internal/v1/channels/{channel_id}/messages/{message_id}");
        stored(self.send(Method::PATCH, &path, Some(patch)).await?)
    }

    async fn delete(&self, channel_id: Snowflake, message_id: Snowflake) -> AppResult<()> {
        let path = format!("internal/v1/channels/{channel_id}/messages/{message_id}");
        self.send(Method::DELETE, &path, None).await.map(|_| ())
    }
}
