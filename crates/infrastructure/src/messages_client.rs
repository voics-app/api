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

    /// Sends the request and returns the status with the JSON body (`Null` when empty).
    /// Server errors become `Unavailable`; other failures are left to the caller.
    async fn exchange(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> AppResult<(reqwest::StatusCode, Value)> {
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
        if !status.is_success() || status == reqwest::StatusCode::NO_CONTENT {
            return Ok((status, Value::Null));
        }
        let body = response
            .json()
            .await
            .map_err(|e| AppError::Unavailable(format!("messages body: {e}")))?;
        Ok((status, body))
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> AppResult<Value> {
        let (status, body) = self.exchange(method, path, body).await?;
        if !status.is_success() {
            return Err(AppError::BadRequest {
                code: 0,
                message: format!("messages responded {status}"),
            });
        }
        Ok(body)
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
    async fn sign_media(&self, urls: Vec<String>) -> AppResult<Vec<String>> {
        let body = self
            .send(
                Method::POST,
                "internal/v1/media/sign",
                Some(serde_json::json!({"urls": urls})),
            )
            .await?;
        serde_json::from_value(body.get("urls").cloned().unwrap_or(Value::Null))
            .map_err(|_| AppError::Unavailable("invalid media signing response".into()))
    }
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

    async fn get(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> AppResult<Option<StoredMessage>> {
        let path = format!("internal/v1/channels/{channel_id}/messages/{message_id}");
        let (status, body) = self.exchange(Method::GET, &path, None).await?;
        match status {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => stored(body).map(Some),
            status => Err(AppError::BadRequest {
                code: 0,
                message: format!("messages responded {status}"),
            }),
        }
    }

    async fn delete(&self, channel_id: Snowflake, message_id: Snowflake) -> AppResult<()> {
        let path = format!("internal/v1/channels/{channel_id}/messages/{message_id}");
        self.send(Method::DELETE, &path, None).await.map(|_| ())
    }
}
