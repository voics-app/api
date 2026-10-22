//! Request metrics middleware: `http_requests_total` and `http_request_duration_seconds`,
//! labelled by method, route template and status class (docs/services/catalog.md).

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;

/// `2xx`-style class for a status code.
pub fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

/// Counts the request and observes its latency once the response is produced.
pub async fn record(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    // The matched template keeps label cardinality bounded; unrouted paths share one label.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());
    let started = Instant::now();
    let response = next.run(request).await;
    let status = status_class(response.status().as_u16());
    metrics::counter!(
        "http_requests_total",
        "method" => method.clone(),
        "route" => route.clone(),
        "status" => status
    )
    .increment(1);
    metrics::histogram!(
        "http_request_duration_seconds",
        "method" => method,
        "route" => route,
        "status" => status
    )
    .record(started.elapsed().as_secs_f64());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use tower::ServiceExt;

    #[test]
    fn classifies_status() {
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(503), "5xx");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn labels_use_route_template_and_status_class() {
        let recorder = PrometheusBuilder::new()
            .set_buckets(&[0.1, 1.0])
            .unwrap()
            .build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let app = Router::new()
            .route("/users/{id}", get(|| async { StatusCode::NOT_FOUND }))
            .layer(axum::middleware::from_fn(record));
        for path in ["/users/1", "/users/2", "/nope"] {
            let _ = app
                .clone()
                .oneshot(HttpRequest::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
        }
        let text = handle.render();
        assert!(
            text.contains(
                r#"http_requests_total{method="GET",route="/users/{id}",status="4xx"} 2"#
            ),
            "{text}"
        );
        assert!(
            text.contains(r#"http_requests_total{method="GET",route="unmatched",status="4xx"} 1"#),
            "{text}"
        );
        assert!(
            text.contains("http_request_duration_seconds_bucket{"),
            "{text}"
        );
    }
}
