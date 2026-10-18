//! Decodes the `data:<mime>;base64,<payload>` images that REST bodies carry.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use application::support::images::ImageUpload;
use application::AppError;

pub fn parse_data_uri(raw: &str) -> Result<ImageUpload, AppError> {
    let invalid =
        || AppError::invalid("image", "INVALID_IMAGE", "Image must be a base64 data URI.");
    let rest = raw.strip_prefix("data:").ok_or_else(invalid)?;
    let (meta, payload) = rest.split_once(";base64,").ok_or_else(invalid)?;
    let bytes = STANDARD.decode(payload.trim()).map_err(|_| invalid())?;
    Ok(ImageUpload {
        bytes,
        content_type: meta.to_owned(),
    })
}

pub fn optional(raw: Option<String>) -> Result<Option<ImageUpload>, AppError> {
    raw.as_deref().map(parse_data_uri).transpose()
}

/// A PATCH field: absent stays absent, `null` clears, a string sets.
pub fn patch(raw: Option<Option<String>>) -> Result<Option<Option<ImageUpload>>, AppError> {
    match raw {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(uri)) => Ok(Some(Some(parse_data_uri(&uri)?))),
    }
}

/// Largest JSON body an image route reads. A 9 MiB image is 12 MiB of base64
/// (the 8 MiB bound is still enforced in the application layer), so this must
/// sit above that to let oversized images reach the `BINARY_TYPE_MAX_SIZE` check.
pub const IMAGE_BODY_LIMIT: usize = 16 * 1024 * 1024;
