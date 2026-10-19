//! MinIO (S3 API) object storage for assets, via presigned PUT requests.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Url};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

use application::ports::AssetStorage;
use application::{AppError, AppResult};

const SIGNATURE_TTL: Duration = Duration::from_secs(300);
const REGION: &str = "us-east-1";

pub struct MinioAssets {
    http: Client,
    bucket: Bucket,
    credentials: Credentials,
}

impl MinioAssets {
    pub fn new(
        endpoint: Url,
        bucket_name: String,
        access_key: String,
        secret_key: String,
    ) -> AppResult<Self> {
        let bucket = Bucket::new(endpoint, UrlStyle::Path, bucket_name, REGION)
            .map_err(AppError::internal)?;
        Ok(Self {
            http: Client::new(),
            bucket,
            credentials: Credentials::new(access_key, secret_key),
        })
    }
}

#[async_trait]
impl AssetStorage for MinioAssets {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> AppResult<()> {
        let url = self
            .bucket
            .put_object(Some(&self.credentials), key)
            .sign(SIGNATURE_TTL);
        let response = self
            .http
            .put(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| AppError::Unavailable(format!("object storage: {e}")))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(AppError::Unavailable(format!(
                "object storage responded {}",
                response.status()
            )))
        }
    }
}
