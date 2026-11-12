//! The optional logo on the single sign-on button. Uploads are decoded and
//! re-encoded to a small PNG, so nothing the uploader wrote is served back.

use std::io::Cursor;

use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::db::DbResult;

pub const MAX_UPLOAD_BYTES: usize = 1024 * 1024;
const MAX_PIXELS: u64 = 4096 * 4096;
const LOGO_HEIGHT: u32 = 96;
const LOGO_MAX_WIDTH: u32 = 4 * LOGO_HEIGHT;

pub const UNSUPPORTED: &str = "Use a PNG, JPEG or WebP image.";
pub const UNREADABLE: &str = "That image could not be read. Try exporting it again as PNG.";
pub const TOO_LARGE: &str = "That image is too large. Use one under 1 MB.";
pub const TOO_MANY_PIXELS: &str = "That image has too many pixels. Use one under 4096×4096.";

/// Never the filename or Content-Type: both are the uploader's claims. SVG
/// is left out because it can carry script.
fn sniff_format(data: &[u8]) -> Option<ImageFormat> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// Pillow's `thumbnail` box fit: aspect kept, never enlarged, each side the
/// whole number nearest the true aspect.
fn thumbnail_size(width: u32, height: u32) -> Option<(u32, u32)> {
    let aspect = f64::from(width) / f64::from(height);
    let pick = |n: f64, error: &dyn Fn(f64) -> f64| {
        let (low, high) = (n.floor(), n.ceil());
        let best = if error(high) < error(low) { high } else { low };
        best.max(1.0) as u32
    };
    let (box_w, box_h) = (f64::from(LOGO_MAX_WIDTH), f64::from(LOGO_HEIGHT));
    let (x, y) = if aspect >= box_w / box_h {
        (
            LOGO_MAX_WIDTH,
            pick(box_w / aspect, &|n| (aspect - box_w / n).abs()),
        )
    } else {
        (
            pick(box_h * aspect, &|n| {
                if n == 0.0 {
                    0.0
                } else {
                    (aspect - n / box_h).abs()
                }
            }),
            LOGO_HEIGHT,
        )
    };
    (x < width || y < height).then_some((x, y))
}

/// The logo as a PNG at most 96 pixels tall, or the message to show.
pub fn normalize(data: &[u8]) -> Result<Vec<u8>, &'static str> {
    if data.len() > MAX_UPLOAD_BYTES {
        return Err(TOO_LARGE);
    }
    let format = sniff_format(data).ok_or(UNSUPPORTED)?;
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4 * 2);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| UNREADABLE)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(TOO_MANY_PIXELS);
    }
    let orientation = decoder.orientation().map_err(|_| UNREADABLE)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|_| UNREADABLE)?;
    image.apply_orientation(orientation);
    let mut image = image::DynamicImage::ImageRgba8(image.to_rgba8());
    if let Some((w, h)) = thumbnail_size(image.width(), image.height()) {
        image = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    }
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| UNREADABLE)?;
    Ok(out.into_inner())
}

pub fn load(conn: &Connection) -> DbResult<Option<Vec<u8>>> {
    conn.query_row("SELECT png FROM auth_oidc_logo WHERE id = 1", [], |row| {
        row.get(0)
    })
    .optional()
}

pub fn current_url(conn: &Connection) -> DbResult<Option<String>> {
    let sha: Option<String> = conn
        .query_row(
            "SELECT sha256 FROM auth_oidc_logo WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(sha.map(|sha| format!("/api/v1/auth/oidc/logo?v={}", &sha[..12.min(sha.len())])))
}

pub fn save(conn: &Connection, png: &[u8], now: f64) -> DbResult<()> {
    let sha: String = Sha256::digest(png)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    conn.execute(
        "INSERT INTO auth_oidc_logo (id, png, sha256, updated_at) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET png = excluded.png, sha256 = excluded.sha256, updated_at = excluded.updated_at",
        params![png, sha, now as i64],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection) -> DbResult<()> {
    conn.execute("DELETE FROM auth_oidc_logo", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn logos_shrink_to_the_button_box_like_pillow() {
        assert_eq!(thumbnail_size(1000, 1000), Some((96, 96)));
        assert_eq!(thumbnail_size(2000, 100), Some((384, 19)));
        assert_eq!(thumbnail_size(50, 40), None);
        let out = normalize(&png(300, 200)).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (144, 96));
    }

    #[test]
    fn only_real_images_are_accepted() {
        assert_eq!(normalize(b"<svg></svg>"), Err(UNSUPPORTED));
        assert_eq!(normalize(b"\x89PNG\r\n\x1a\nnot really"), Err(UNREADABLE));
        assert_eq!(normalize(&vec![0u8; MAX_UPLOAD_BYTES + 1]), Err(TOO_LARGE));
    }
}
