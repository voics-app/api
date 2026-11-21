//! Optional Tenor-compatible GIF provider. Enable only with a provider whose
//! agreement permits server-side requests and response caching.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{error::ApiError, extract::Caller, state::AppState};
use application::AppError;

#[derive(Clone)]
pub struct GifProvider {
    base: Option<reqwest::Url>,
    key: String,
    media_hosts: Vec<String>,
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
        let media_hosts: Vec<String> = std::env::var("GIF_MEDIA_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Self {
            base: base.filter(|_| !key.is_empty() && !media_hosts.is_empty()),
            key,
            media_hosts,
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
        let raw: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Unavailable("invalid GIF response".into()))?;
        let value = sanitize(endpoint, &raw, &self.key, &self.media_hosts);
        let mut cache = self.cache.lock().expect("GIF cache");
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(cache_key, (Instant::now(), value.clone()));
        Ok(value)
    }
}

/// Only expose fields consumed by the picker. Never forward opaque provider metadata.
fn allowed_url(text: &str, key: &str, hosts: &[String]) -> bool {
    if text.len() > 2048 || text.contains(key) {
        return false;
    }
    reqwest::Url::parse(text).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
            && !u.path().contains('%')
            && u.host_str().is_some_and(|h| hosts.iter().any(|a| a == h))
            && !u
                .query_pairs()
                .any(|(k, v)| k.contains(key) || v.contains(key))
    })
}
fn sanitize(endpoint: &str, raw: &Value, key: &str, hosts: &[String]) -> Value {
    fn safe_text<'a>(v: Option<&'a Value>, key: &str, max: usize) -> Option<&'a str> {
        v.and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= max && !s.contains(key))
    }
    fn safe_url(v: Option<&Value>, key: &str, hosts: &[String]) -> Option<String> {
        let text = safe_text(v, key, 2048)?;
        if !allowed_url(text, key, hosts) {
            return None;
        }
        Some(text.to_owned())
    }
    if endpoint == "categories" {
        let tags: Vec<_> = raw.get("tags").and_then(Value::as_array).into_iter().flatten().take(50).filter_map(|tag| {
            Some(serde_json::json!({"name": safe_text(tag.get("name"), key, 100)?, "searchterm": safe_text(tag.get("searchterm"), key, 100)?}))
        }).collect();
        return serde_json::json!({"tags": tags});
    }
    let results: Vec<_> = raw.get("results").and_then(Value::as_array).into_iter().flatten().take(24).filter_map(|item| {
        let id = safe_text(item.get("id"), key, 100)?;
        let formats = item.get("media_formats")?;
        let gif = safe_url(formats.get("gif").and_then(|v| v.get("url")), key, hosts)?;
        let tiny = safe_url(formats.get("tinygif").and_then(|v| v.get("url")), key, hosts).unwrap_or_else(|| gif.clone());
        let description = safe_text(item.get("content_description"), key, 200).unwrap_or("GIF");
        Some(serde_json::json!({"id": id, "content_description": description, "media_formats": {"gif": {"url": gif}, "tinygif": {"url": tiny}}}))
    }).collect();
    serde_json::json!({"results": results})
}

#[derive(Deserialize)]
struct Search {
    q: String,
}

async fn trending(
    State(state): State<AppState>,
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        sign_results(&state, provider.get("featured", None).await?).await?,
    ))
}
async fn categories(
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(provider.get("categories", None).await?))
}
async fn search(
    State(state): State<AppState>,
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
    Ok(Json(
        sign_results(&state, provider.get("search", Some(q)).await?).await?,
    ))
}

async fn sign_results(state: &AppState, mut value: Value) -> Result<Value, ApiError> {
    let Some(results) = value.get_mut("results").and_then(Value::as_array_mut) else {
        return Ok(value);
    };
    let urls: Vec<String> = results
        .iter()
        .filter_map(|v| {
            v.pointer("/media_formats/tinygif/url")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    if urls.is_empty() {
        return Ok(value);
    }
    let signed = state.ports.messages.sign_media(urls).await?;
    if signed.len() != results.len() {
        return Err(AppError::Unavailable("invalid media signing response".into()).into());
    }
    for (item, signed) in results.iter_mut().zip(signed) {
        item["media_formats"]["tinygif"]["proxy_url"] = Value::String(signed);
    }
    Ok(value)
}

#[derive(Deserialize)]
struct PreviewRequest {
    urls: Vec<String>,
}
async fn previews(
    State(state): State<AppState>,
    Caller(_): Caller,
    Extension(provider): Extension<GifProvider>,
    Json(body): Json<PreviewRequest>,
) -> Result<Json<Value>, ApiError> {
    if provider.base.is_none()
        || body.urls.len() > 48
        || body
            .urls
            .iter()
            .any(|url| !allowed_url(url, &provider.key, &provider.media_hosts))
    {
        return Err(AppError::invalid(
            "urls",
            "INVALID_URL",
            "GIF URLs must use a configured media host",
        )
        .into());
    }
    let signed = state.ports.messages.sign_media(body.urls).await?;
    Ok(Json(serde_json::json!({"urls": signed})))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/gifs/trending", get(trending))
        .route("/gifs/search", get(search))
        .route("/gifs/categories", get(categories))
        .route("/gifs/previews", post(previews))
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
                Json(serde_json::json!({"path": "fixture-key", "results": [{"id": "1", "secret": "fixture-key", "media_formats": {"gif": {"url": "https://example.test/1.gif"}}}]}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = GifProvider {
            base: Some(format!("http://{addr}/v2/").parse().unwrap()),
            key: "fixture-key".into(),
            media_hosts: vec!["example.test".into()],
            client: reqwest::Client::new(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        };
        assert_eq!(
            provider.get("search", Some("happy cat")).await.unwrap()["results"][0]["id"],
            "1"
        );
        provider.get("search", Some("happy cat")).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!provider
            .get("search", Some("happy cat"))
            .await
            .unwrap()
            .to_string()
            .contains("fixture-key"));
        let categories = sanitize(
            "categories",
            &serde_json::json!({"path": "?key=fixture-key", "tags": [{"name": "Cats", "searchterm": "cats", "path": "fixture-key"}]}),
            "fixture-key",
            &["example.test".into()],
        );
        assert!(!categories.to_string().contains("fixture-key"));
        assert!(sanitize("search", &serde_json::json!({"results": [{"id": "1", "media_formats": {"gif": {"url": "https://example.test/a.gif?key=fixture%2Dkey"}}}]}), "fixture-key", &["example.test".into()])["results"].as_array().unwrap().is_empty());
        let disabled = GifProvider {
            base: None,
            ..provider
        };
        assert!(disabled.get("search", Some("happy cat")).await.is_err());
        server.abort();
    }
}
