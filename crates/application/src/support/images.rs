use crate::context::Ports;
use crate::error::{AppError, AppResult};

/// 10 MiB upper bound for avatars, icons and banners.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

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
            _ => Err(AppError::invalid(
                "image",
                "IMAGE_INVALID",
                "Unsupported image type.",
            )),
        }
    }

    pub fn is_animated(&self) -> bool {
        self.content_type == "image/gif"
    }

    fn ensure_size(&self, max: usize) -> AppResult<()> {
        if self.bytes.is_empty() || self.bytes.len() > max {
            return Err(AppError::invalid(
                "image",
                "BINARY_TYPE_MAX_SIZE",
                "Image is empty or too large.",
            ));
        }
        Ok(())
    }
}

/// Stores an image and returns its hash. The CDN is plain object storage that cannot convert
/// formats, so the key follows Discord's URL convention: animated images get an `a_` hash and live
/// at `{prefix}/{hash}.gif`, every static image lives at `{prefix}/{hash}.png` (stored with its real
/// Content-Type, which browsers render regardless of the key's extension).
pub async fn store_hashed(ports: &Ports, prefix: &str, image: ImageUpload) -> AppResult<String> {
    image.ensure_size(MAX_IMAGE_BYTES)?;
    image.extension()?;
    let (hash, key) = hashed_key(prefix, &ports.secrets.token(16), &image);
    ports
        .assets
        .put(&key, image.bytes, &image.content_type)
        .await?;
    Ok(hash)
}

fn hashed_key(prefix: &str, token: &str, image: &ImageUpload) -> (String, String) {
    if image.is_animated() {
        let hash = format!("a_{token}");
        let key = format!("{prefix}/{hash}.gif");
        (hash, key)
    } else {
        (token.to_owned(), format!("{prefix}/{token}.png"))
    }
}

/// Stores an image at an exact key (custom emoji use their id).
pub async fn store_at(
    ports: &Ports,
    key_without_extension: &str,
    image: ImageUpload,
    max: usize,
) -> AppResult<()> {
    image.ensure_size(max)?;
    let extension = image.extension()?;
    let key = format!("{key_without_extension}.{extension}");
    ports
        .assets
        .put(&key, image.bytes, &image.content_type)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(content_type: &str) -> ImageUpload {
        ImageUpload {
            bytes: vec![1],
            content_type: content_type.to_owned(),
        }
    }

    #[test]
    fn animated_images_get_an_a_hash_and_a_gif_key() {
        assert_eq!(
            hashed_key("icons/1", "abc", &image("image/gif")),
            ("a_abc".to_owned(), "icons/1/a_abc.gif".to_owned())
        );
    }

    #[test]
    fn static_images_live_under_png_whatever_their_encoding() {
        for content_type in ["image/png", "image/jpeg", "image/webp"] {
            assert_eq!(
                hashed_key("avatars/1", "abc", &image(content_type)),
                ("abc".to_owned(), "avatars/1/abc.png".to_owned())
            );
        }
    }
}
