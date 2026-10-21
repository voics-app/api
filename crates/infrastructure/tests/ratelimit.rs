//! Token-bucket limiter against a real Valkey, and the 429 contract through
//! the router.

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tower::ServiceExt;

use application::ports::RateLimiter;
use infrastructure::ratelimit::ValkeyRateLimiter;
use interface::state::{AppState, RatePolicy};
use support::{jwt_keys, ports, start_database};

async fn start_valkey() -> (testcontainers::ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("valkey/valkey", "8-alpine")
        .with_exposed_port(ContainerPort::Tcp(6379))
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_startup_timeout(Duration::from_secs(300))
        .start()
        .await
        .expect("start valkey");
    let port = container
        .get_host_port_ipv4(6379)
        .await
        .expect("mapped port");
    (container, format!("redis://127.0.0.1:{port}"))
}

#[tokio::test]
async fn token_bucket_admits_capacity_then_throttles_per_key() {
    let (_valkey, url) = start_valkey().await;
    let limiter = ValkeyRateLimiter::connect(&url).await.expect("connect");
    let window = Duration::from_secs(60);

    for expected_remaining in [2, 1, 0] {
        let decision = limiter.hit("rl:test:a", 3, window).await.expect("hit");
        assert!(decision.allowed);
        assert_eq!(decision.remaining, expected_remaining);
    }
    let denied = limiter.hit("rl:test:a", 3, window).await.expect("hit");
    assert!(!denied.allowed);
    assert!(denied.retry_after > Duration::ZERO);
    assert!(denied.retry_after <= window);

    let other = limiter.hit("rl:test:b", 3, window).await.expect("hit");
    assert!(other.allowed, "buckets are independent");
}

#[tokio::test]
async fn throttled_requests_get_429_with_discord_headers() {
    let (_valkey, url) = start_valkey().await;
    let database = start_database().await;
    let (private, public) = jwt_keys();
    let app = interface::router(AppState {
        ports: ports(&database.pool, &public, &private),
        internal_token: support::http::INTERNAL_TOKEN.into(),
        rate_limiter: Arc::new(ValkeyRateLimiter::connect(&url).await.expect("connect")),
        rate_policy: RatePolicy {
            limit: 2,
            window: Duration::from_secs(60),
        },
    });

    let mut statuses = Vec::new();
    let mut last = None;
    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/users/@me")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router responds");
        statuses.push(response.status());
        last = Some(response);
    }
    assert_eq!(
        statuses[0],
        StatusCode::UNAUTHORIZED,
        "allowed requests reach the handler"
    );
    assert_eq!(statuses[2], StatusCode::TOO_MANY_REQUESTS);

    let throttled = last.expect("third response");
    assert!(throttled.headers().contains_key("retry-after"));
    assert_eq!(throttled.headers()["x-ratelimit-limit"], "2");
    assert_eq!(throttled.headers()["x-ratelimit-remaining"], "0");
    assert_eq!(throttled.headers()["x-ratelimit-bucket"], "GET /users/@me");
    assert!(throttled.headers().contains_key("x-ratelimit-reset-after"));
    assert!(throttled.headers().contains_key("x-ratelimit-reset"));
}
