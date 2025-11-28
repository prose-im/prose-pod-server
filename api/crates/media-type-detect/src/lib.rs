// media-type-detect
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

/// NOTE: If type safety becomes a need, add `mime` as an optional dependency.
pub type MediaType = &'static str;

pub mod media_type {
    use super::MediaType;

    pub const IMAGE_PNG: MediaType = "image/png";
    pub const IMAGE_GIF: MediaType = "image/gif";
    pub const IMAGE_JPEG: MediaType = "image/jpeg";
}

pub const SUPPORTED_IMAGE_MEDIA_TYPES: [MediaType; 3] = [
    media_type::IMAGE_PNG,
    media_type::IMAGE_GIF,
    media_type::IMAGE_JPEG,
];

// NOTE: See [List of file signatures | Wikipedia](https://en.wikipedia.org/wiki/List_of_file_signatures).
const MAGIC_PREFIX_PNG: [u8; 8] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
];
const MAGIC_PREFIX_GIF: [u8; 4] = [
    0x47, 0x49, 0x46, 0x38,
];
const MAGIC_PREFIX_JPEG: [u8; 3] = [0xff, 0xd8, 0xff];

#[must_use]
pub fn detect_image_media_type(data: impl AsRef<[u8]>) -> Option<MediaType> {
    let data = data.as_ref();
    if data.starts_with(&MAGIC_PREFIX_PNG) {
        Some(media_type::IMAGE_PNG)
    } else if data.starts_with(&MAGIC_PREFIX_GIF) {
        Some(media_type::IMAGE_GIF)
    } else if data.starts_with(&MAGIC_PREFIX_JPEG) {
        Some(media_type::IMAGE_JPEG)
    } else {
        None
    }
}

pub struct UnusupportedMediaType;

#[must_use]
pub fn is_media_type(
    data: impl AsRef<[u8]>,
    media_type: &MediaType,
) -> Result<bool, UnusupportedMediaType> {
    let data = data.as_ref();
    match media_type {
        &media_type::IMAGE_PNG => Ok(data.starts_with(&MAGIC_PREFIX_PNG)),
        &media_type::IMAGE_GIF => Ok(data.starts_with(&MAGIC_PREFIX_GIF)),
        &media_type::IMAGE_JPEG => Ok(data.starts_with(&MAGIC_PREFIX_JPEG)),
        _ => Err(UnusupportedMediaType),
    }
}

#[cfg(test)]
mod tests {
    use super::SUPPORTED_IMAGE_MEDIA_TYPES;

    /// We use this in errors, let’s just check the format.
    #[test]
    fn test_supported_types_display() {
        assert_eq!(
            format!("{SUPPORTED_IMAGE_MEDIA_TYPES:?}").as_str(),
            r#"["image/png", "image/gif", "image/jpeg"]"#,
        );
    }
}
