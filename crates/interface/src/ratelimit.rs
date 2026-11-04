//! Rate limiting for the public API (P9): per identity and route bucket,
//! with Discord's `X-RateLimit-*` headers and a 429 body when exhausted.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use sha2::{Digest, Sha256};

use application::ports::RateLimitDecision;

use crate::state::AppState;

/// Path segments that vary per resource are folded so every resource shares one bucket.
pub fn bucket_for(method: &Method, path: &str) -> String {
    let folded: Vec<&str> = path
        .split('/')
        .map(|segment| {
            if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
                ":id"
            } else if segment.len() > 40 {
                ":token"
            } else {
                segment
            }
        })
        .collect();
    format!("{method} {}", folded.join("/"))
}

/// Callers are identified by a hash of their credential, or by their forwarded address.
pub fn identity_of(headers: &HeaderMap) -> String {
    let source = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|credential| format!("auth:{credential}"))
        .or_else(|| {
            headers
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok())
                .and_then(|list| list.split(',').next())
                .map(|ip| format!("ip:{}", ip.trim()))
        })
        .unwrap_or_else(|| "anonymous".to_owned());
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

fn set_header(headers: &mut HeaderMap, name: &'static str, value: String) {
    if let Ok(value) = HeaderValue::from_str(&value) {
        headers.insert(HeaderName::from_static(name), value);
    }
}

fn apply_headers(headers: &mut HeaderMap, decision: &RateLimitDecision, bucket: &str) {
    set_header(headers, "x-ratelimit-limit", decision.limit.to_string());
    set_header(
        headers,
        "x-ratelimit-remaining",
        decision.remaining.to_string(),
    );
    set_header(
        headers,
        "x-ratelimit-reset",
        format!(
            "{:.3}",
            epoch_seconds() + decision.reset_after.as_secs_f64()
        ),
    );
    set_header(
        headers,
        "x-ratelimit-reset-after",
        format!("{:.3}", decision.reset_after.as_secs_f64()),
    );
    set_header(headers, "x-ratelimit-bucket", bucket.to_owned());
}

fn throttled(decision: &RateLimitDecision, bucket: &str) -> Response {
    let retry_seconds = decision.retry_after.as_secs_f64();
    let body = json!({
        "code": 429,
        "message": "You are being rate limited.",
        "retry_after": retry_seconds,
        "global": false,
    });
    let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    apply_headers(response.headers_mut(), decision, bucket);
    let whole_seconds = decision.retry_after.as_secs_f64().ceil().max(1.0) as u64;
    set_header(
        response.headers_mut(),
        RETRY_AFTER.as_str(),
        whole_seconds.to_string(),
    );
    response
}

/// `rate_limit_decisions_total{scope,result}`: `result` is `allowed`, `limited` or `error` (limiter
/// outage, request let through).
fn record_decision(scope: &'static str, result: &'static str) {
    metrics::counter!("rate_limit_decisions_total", "scope" => scope, "result" => result)
        .increment(1);
}

/// Middleware for the public API. The limiter falls back to per-pod memory buckets when its store fails
/// (`FallbackRateLimiter`); the error arm below is only reached by a limiter without that fallback.
pub async fn enforce(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let bucket = bucket_for(request.method(), request.uri().path());
    let key = format!("rl:{}:{bucket}", identity_of(request.headers()));
    let policy = state.rate_policy;
    match state
        .rate_limiter
        .hit(&key, policy.limit, policy.window)
        .await
    {
        Ok(decision) if !decision.allowed => {
            record_decision("general", "limited");
            throttled(&decision, &bucket)
        }
        Ok(decision) => {
            record_decision("general", "allowed");
            let mut response = next.run(request).await;
            apply_headers(response.headers_mut(), &decision, &bucket);
            response
        }
        Err(error) => {
            record_decision("general", "error");
            tracing::warn!(%error, "rate limiter unavailable; request allowed");
            next.run(request).await
        }
    }
}

/// A stricter per-caller bucket on top of the general limiter, for endpoints
/// that send mail or accept guessable secrets. Falls back like [`enforce`].
pub async fn strict(
    state: &AppState,
    headers: &HeaderMap,
    name: &str,
    limit: u32,
    window: std::time::Duration,
) -> Result<(), crate::error::ApiError> {
    let key = format!("rl-strict:{}:{name}", identity_of(headers));
    match state.rate_limiter.hit(&key, limit, window).await {
        Ok(decision) if !decision.allowed => {
            record_decision("strict", "limited");
            Err(application::AppError::RateLimited {
                retry_after_secs: decision.retry_after.as_secs().max(1),
            }
            .into())
        }
        Ok(_) => {
            record_decision("strict", "allowed");
            Ok(())
        }
        Err(error) => {
            record_decision("strict", "error");
            tracing::warn!(%error, "rate limiter unavailable; request allowed");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_are_counted_by_scope_and_result() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        record_decision("general", "limited");
        record_decision("general", "limited");
        assert!(handle
            .render()
            .contains(r#"rate_limit_decisions_total{scope="general",result="limited"} 2"#));
    }

    #[test]
    fn numeric_and_long_segments_share_a_bucket() {
        assert_eq!(
            bucket_for(&Method::GET, "/api/v1/channels/123/messages"),
            "GET /api/v1/channels/:id/messages"
        );
        assert_eq!(
            bucket_for(&Method::GET, "/api/v1/channels/456/messages"),
            "GET /api/v1/channels/:id/messages"
        );
        let token = "a".repeat(64);
        assert_eq!(
            bucket_for(&Method::POST, &format!("/api/v1/webhooks/9/{token}")),
            "POST /api/v1/webhooks/:id/:token"
        );
    }

    #[test]
    fn identity_prefers_the_credential_over_the_address() {
        let mut with_auth = HeaderMap::new();
        with_auth.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
        let mut forwarded = HeaderMap::new();
        forwarded.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 10.0.0.1"),
        );
        assert_ne!(identity_of(&with_auth), identity_of(&forwarded));
        assert_eq!(
            identity_of(&HeaderMap::new()),
            identity_of(&HeaderMap::new())
        );
        assert_eq!(
            identity_of(&with_auth).len(),
            64,
            "identities are hashed, never raw"
        );
    }
}
