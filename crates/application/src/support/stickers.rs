//! Bounds and identifies sticker assets before upload.
use super::images::ImageUpload;
use crate::error::{AppError, AppResult};

pub const MAX_STICKER_BYTES: usize = 512 * 1024;
pub const STICKER_WIDTH: u32 = 320;
pub const STICKER_HEIGHT: u32 = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickerFormat {
    Png = 1,
    Apng = 2,
    Lottie = 3,
    Gif = 4,
}

fn invalid() -> AppError {
    AppError::invalid(
        "image",
        "INVALID_STICKER",
        "Invalid sticker image or dimensions.",
    )
}

/// Identifies PNG/APNG, GIF and Lottie by their contents and enforces the upload limits.
pub fn validate(image: &ImageUpload) -> AppResult<StickerFormat> {
    if image.bytes.is_empty() || image.bytes.len() > MAX_STICKER_BYTES {
        return Err(invalid());
    }
    let (format, width, height) = match image.content_type.as_str() {
        "image/png" => png(&image.bytes)?,
        "image/gif" => gif(&image.bytes)?,
        "application/json" => lottie(&image.bytes)?,
        _ => return Err(invalid()),
    };
    if width != STICKER_WIDTH || height != STICKER_HEIGHT {
        return Err(invalid());
    }
    if format != StickerFormat::Lottie {
        let image_format = if format == StickerFormat::Gif {
            image::ImageFormat::Gif
        } else {
            image::ImageFormat::Png
        };
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(&image.bytes), image_format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(STICKER_WIDTH);
        limits.max_image_height = Some(STICKER_HEIGHT);
        limits.max_alloc = Some(8 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().map_err(|_| invalid())?;
    }
    Ok(format)
}

fn png(bytes: &[u8]) -> AppResult<(StickerFormat, u32, u32)> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(invalid());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| invalid())?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| invalid())?);
    let mut offset = 8usize;
    let mut animated = false;
    let mut complete = false;
    while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .map_err(|_| invalid())?,
        ) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .ok_or_else(invalid)?;
        if end > bytes.len() {
            return Err(invalid());
        }
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"acTL" {
            animated = true;
        }
        if kind == b"IEND" {
            complete = length == 0 && end == bytes.len();
            break;
        }
        offset = end;
    }
    if !complete {
        return Err(invalid());
    }
    Ok((
        if animated {
            StickerFormat::Apng
        } else {
            StickerFormat::Png
        },
        width,
        height,
    ))
}

fn gif(bytes: &[u8]) -> AppResult<(StickerFormat, u32, u32)> {
    if bytes.len() < 14
        || !matches!(&bytes[..6], b"GIF87a" | b"GIF89a")
        || bytes.last() != Some(&0x3b)
    {
        return Err(invalid());
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
    let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
    Ok((StickerFormat::Gif, width, height))
}

fn lottie(bytes: &[u8]) -> AppResult<(StickerFormat, u32, u32)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let width = value
        .get("w")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(invalid)?;
    let height = value
        .get("h")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(invalid)?;
    if contains_external_asset_or_expression(&value) {
        return Err(invalid());
    }
    if !value.get("layers").is_some_and(|v| v.is_array())
        || !value.get("v").is_some_and(|v| v.is_string())
    {
        return Err(invalid());
    }
    Ok((StickerFormat::Lottie, width, height))
}

fn contains_external_asset_or_expression(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, child)| {
            ((key == "p" || key == "u" || key == "x") && child.is_string())
                || contains_external_asset_or_expression(child)
        }),
        serde_json::Value::Array(values) => {
            values.iter().any(contains_external_asset_or_expression)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mime_spoof_and_wrong_dimensions() {
        let image = ImageUpload {
            bytes: br#"{"v":"1","w":319,"h":320,"layers":[]}"#.to_vec(),
            content_type: "application/json".into(),
        };
        assert!(validate(&image).is_err());
        let image = ImageUpload {
            bytes: image.bytes,
            content_type: "image/png".into(),
        };
        assert!(validate(&image).is_err());
    }

    #[test]
    fn accepts_bounded_lottie_and_rejects_oversize() {
        let mut image = ImageUpload {
            bytes: br#"{"v":"1","w":320,"h":320,"layers":[]}"#.to_vec(),
            content_type: "application/json".into(),
        };
        assert_eq!(validate(&image).unwrap(), StickerFormat::Lottie);
        image.bytes.resize(MAX_STICKER_BYTES + 1, b' ');
        assert!(validate(&image).is_err());
    }
}
