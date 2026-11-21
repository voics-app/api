//! Optional cross-repository browser acceptance: real Chromium credentials and this API verifier.
mod support;

#[tokio::test]
#[ignore = "Requires a client/website checkout, Chrome and local HTTPS certificate; see client passkey runbook"]
async fn browser_credentials_are_verified_by_the_api() {
    let client = std::env::var("PASSKEY_BROWSER_CLIENT").expect("client checkout path");
    let f = support::passkeys::fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, f.app).await.unwrap() });
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .arg(std::path::Path::new(&client).join("node_modules/@playwright/test/cli.js"))
            .args(["test", "--config", "playwright.passkeys.config.ts"])
            .current_dir(std::path::Path::new(&client).join("apps/desktop"))
            .env("PASSKEY_BROWSER_API", format!("http://{address}"))
            .env("PASSKEY_BROWSER_ACCESS", f.access)
            .status()
            .expect("run headless browser acceptance")
    })
    .await
    .unwrap();
    server.abort();
    drop(f.database);
    assert!(status.success(), "browser acceptance failed");
}
