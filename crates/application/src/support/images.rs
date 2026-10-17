use crate::context::Ports;
use crate::error::{AppError, AppResult};

/// 8 MiB upper bound for avatars and icons.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// A decoded image from a Discord-style data URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageUpload {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl ImageUpload {
    pub fn extension(&self) -> AppResult<&'static str> {
        match self.content_type.as_str() {
            "image/png" => Ok("png"),
            "image/jpeg" => Ok("jpg"),
            "image/gif" => Ok("gif"),
            "image/webp" => Ok("webp"),
            _ => Err(AppError::invalid("image", "IMAGE_INVALID", "Unsupported image type.")),
        }
    }

    pub fn is_animated(&self) -> bool {
        self.content_type == "image/gif"
    }

    fn ensure_size(&self, max: usize) -> AppResult<()> {
        if self.bytes.is_empty() || self.bytes.len() > max {
            return Err(AppError::invalid("image", "BINARY_TYPE_MAX_SIZE", "Image is empty or too large."));
        }
        Ok(())
    }
}

/// Stores an image under `{prefix}/{hash}.{ext}` and returns the hash.
pub async fn store_hashed(ports: &Ports, prefix: &str, image: ImageUpload) -> AppResult<String> {
    image.ensure_size(MAX_IMAGE_BYTES)?;
    let extension = image.extension()?;
    let hash = ports.secrets.token(16);
    let key = format!("{prefix}/{hash}.{extension}");
    ports.assets.put(&key, image.bytes, &image.content_type).await?;
    Ok(hash)
}

/// Stores an image at an exact key (custom emoji use their id).
pub async fn store_at(ports: &Ports, key_without_extension: &str, image: ImageUpload, max: usize) -> AppResult<()> {
    image.ensure_size(max)?;
    let extension = image.extension()?;
    let key = format!("{key_without_extension}.{extension}");
    ports.assets.put(&key, image.bytes, &image.content_type).await
}
