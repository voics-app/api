//! Real enrollment interleaved with a profile mutation at its commit boundary.
mod support;
use application::{
    ports::UnitOfWork,
    write_set::{Mutation, WriteSet},
    AppResult,
};
use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use serde_json::json;
use std::sync::Arc;
use support::http::call;
use support::passkeys::{app, enroll, fixture};
use tokio::sync::Notify;

struct PausedProfileCommit {
    inner: Arc<dyn UnitOfWork>,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl UnitOfWork for PausedProfileCommit {
    async fn commit(&self, writes: WriteSet) -> AppResult<()> {
        if writes
            .mutations
            .iter()
            .any(|change| matches!(change, Mutation::UpdateUser(_)))
        {
            self.arrived.notify_one();
            self.release.notified().await;
        }
        self.inner.commit(writes).await
    }
}

#[tokio::test]
async fn profile_http_response_keeps_mfa_enabled_when_enrollment_commits_first() {
    let f = fixture().await;
    let arrived = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut ports = f.ports.clone();
    ports.unit_of_work = Arc::new(PausedProfileCommit {
        inner: ports.unit_of_work,
        arrived: arrived.clone(),
        release: release.clone(),
    });
    let paused_app = app(ports);
    let access = f.access.clone();
    let update = tokio::spawn(async move {
        call(
            &paused_app,
            Method::PATCH,
            "/api/v1/users/@me",
            &access,
            Some(json!({"global_name":"Updated profile"})),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), arrived.notified())
        .await
        .expect("profile read its snapshot");
    enroll(&f).await;
    release.notify_one();
    let (status, response) = update.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response["mfa_enabled"], true,
        "HTTP response must use committed security state"
    );
    assert_eq!(response["global_name"], "Updated profile");
}
