//! Optional Tenor-compatible GIF provider. Enable only with a provider whose
//! agreement permits server-side requests and response caching.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{extract::Query, routing::get, Extension, Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::{error::ApiError, extract::Caller, state::AppState};
use application::AppError;

#[derive(Clone)]
pub struct GifProvider {
    base: Option<reqwest::Url>,
    key: String,
    client: reqwest::Client,
    cache: Arc<Mutex<HashMap<String, (Instant, Value)>>>,
}

impl GifProvider {
    pub fn from_env() -> Self {
        let base = std::env::var("GIF_PROVIDER_URL")
            .ok()
            .and_then(|s| reqwest::Url::parse(&s).ok())
            .filter(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none()
            });
        let key = std::env::var("GIF_PROVIDER_KEY").unwrap_or_default();
        Self {
            base: base.filter(|_| !key.is_empty()),
            key,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("GIF HTTP client"),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn get(&self, endpoint: &str, q: Option<&str>) -> Result<Value, ApiError> {
        let base = self
            .base
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("GIF provider is not configured".into()))?;
        let cache_key = format!("{endpoint}:{}", q.unwrap_or(""));
        if let Some((time, value)) = self.cache.lock().expect("GIF cache").get(&cache_key) {
            if time.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            }
        }
        let mut url = base.clone();
        url.path_segments_mut()
            .map_err(|_| AppError::Unavailable("invalid GIF provider URL".into()))?
            .pop_if_empty()
            .push(endpoint);
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("key", &self.key)
                .append_pair("limit", "24");
            if let Some(q) = q {
                query.append_pair("q", q);
            }
        }
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| AppError::Unavailable("GIF provider request failed".into()))?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n > 1_000_000)
        {
            return Err(AppError::Unavailable("GIF provider returned an error".into()).into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::Unavailable("GIF provider response failed".into()))?
        {
            if bytes.len() + chunk.len() > 1_000_000 {
                return Err(AppError::Unavailable("GIF response too large".into()).into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Unavailable("invalid GIF response".into()))?;
        let mut cache = self.cache.lock().expect("GIF cache");
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(cache_key, (Instant::now(), value.clone()));
        Ok(value)
    }
}

#[derive(Deserialize)]
struct Search {
    q: String,
}

async fn trending(
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(provider.get("featured", None).await?))
}
async fn categories(
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(provider.get("categories", None).await?))
}
async fn search(
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
    Query(params): Query<Search>,
) -> Result<Json<Value>, ApiError> {
    let q = params.q.trim();
    if q.is_empty() || q.chars().count() > 100 {
        return Err(
            AppError::invalid("q", "INVALID_QUERY", "Search must be 1–100 characters").into(),
        );
    }
    Ok(Json(provider.get("search", Some(q)).await?))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/gifs/trending", get(trending))
        .route("/gifs/search", get(search))
        .route("/gifs/categories", get(categories))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn fixture_provider_is_cached_and_disabled_without_configuration() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = Router::new().route("/v2/search", get(move |Query(q): Query<HashMap<String, String>>| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                assert_eq!(q.get("key").map(String::as_str), Some("fixture-key"));
                assert_eq!(q.get("q").map(String::as_str), Some("happy cat"));
                Json(serde_json::json!({"results": [{"id": "1", "media_formats": {"gif": {"url": "https://example.test/1.gif"}}}]}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = GifProvider {
            base: Some(format!("http://{addr}/v2/").parse().unwrap()),
            key: "fixture-key".into(),
            client: reqwest::Client::new(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        };
        assert_eq!(
            provider.get("search", Some("happy cat")).await.unwrap()["results"][0]["id"],
            "1"
        );
        provider.get("search", Some("happy cat")).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let disabled = GifProvider {
            base: None,
            ..provider
        };
        assert!(disabled.get("search", Some("happy cat")).await.is_err());
        server.abort();
    }
}
