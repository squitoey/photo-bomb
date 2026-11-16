//! Decode a photo once and derive everything the index needs from it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use image::metadata::Orientation;
use image::{DynamicImage, ImageReader};

use crate::hash;

pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "avif", "cr2",
    "cr3", "nef", "arw", "dng", "raf", "orf", "rw2",
];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "m4v"];
/// Formats the Rust decoders do not handle. On macOS these go through `sips`.
const SIPS_EXTS: &[&str] = &[
    "heic", "heif", "avif", "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2",
];

pub fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_image(path: &Path) -> bool {
    IMAGE_EXTS.contains(&ext_of(path).as_str())
}

/// True when a browser cannot show this format directly (HEIC, AVIF, RAW).
pub fn needs_conversion(path: &Path) -> bool {
    SIPS_EXTS.contains(&ext_of(path).as_str())
}

/// Converts `path` to a JPEG no larger than `max_px` on its long edge.
pub fn convert_to_jpeg(path: &Path, out: &Path, max_px: u32) -> Result<()> {
    let res = Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-Z", &max_px.to_string()])
        .arg(path)
        .arg("--out")
        .arg(out)
        .output()
        .context("could not run sips")?;
    if !res.status.success() || !out.exists() {
        bail!("sips could not convert the file");
    }
    Ok(())
}

pub fn is_media(path: &Path) -> bool {
    let e = ext_of(path);
    IMAGE_EXTS.contains(&e.as_str()) || VIDEO_EXTS.contains(&e.as_str())
}

pub struct Decoded {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub taken_at: Option<i64>,
    pub phash: u64,
    pub dhash: u64,
}

/// Decodes `path`, writes a thumbnail to `thumb_path`, returns metadata and hashes.
pub fn process(path: &Path, thumb_path: &Path, scratch: &Path) -> Result<Decoded> {
    let ext = ext_of(path);
    let (img, width, height) = if SIPS_EXTS.contains(&ext.as_str()) {
        decode_with_sips(path, scratch)?
    } else {
        let img = ImageReader::open(path)?
            .with_guessed_format()?
            .decode()
            .context("decode failed")?;
        let img = apply_exif_orientation(img, path);
        let (w, h) = (img.width(), img.height());
        (img, w, h)
    };
    if width == 0 || height == 0 {
        bail!("empty image");
    }

    // Everything else works from a small copy.
    let small = img.thumbnail(320, 320);
    drop(img);
    small
        .to_rgb8()
        .save_with_format(thumb_path, image::ImageFormat::Jpeg)
        .context("thumbnail write failed")?;

    Ok(Decoded {
        format: ext,
        width,
        height,
        taken_at: exif_taken_at(path),
        phash: hash::phash(&small),
        dhash: hash::dhash(&small),
    })
}

fn apply_exif_orientation(mut img: DynamicImage, path: &Path) -> DynamicImage {
    if let Some(o) =
        exif_u32(path, exif::Tag::Orientation).and_then(|v| Orientation::from_exif(v as u8))
    {
        img.apply_orientation(o);
    }
    img
}

/// Converts to a reduced JPEG with `sips` and reads the original pixel size.
/// sips bakes the orientation into the pixels, so none is applied afterwards.
fn decode_with_sips(path: &Path, scratch: &Path) -> Result<(DynamicImage, u32, u32)> {
    if !cfg!(target_os = "macos") {
        bail!("this format is only supported on macOS in this build");
    }
    let tmp: PathBuf = scratch.join(format!(
        "sips-{}-{:?}.jpg",
        std::process::id(),
        std::thread::current().id()
    ));
    let out = Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-Z", "1024"])
        .arg(path)
        .arg("--out")
        .arg(&tmp)
        .output()
        .context("could not run sips")?;
    if !out.status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        bail!(
            "sips could not convert the file: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let img = ImageReader::open(&tmp)
        .and_then(|r| r.with_guessed_format())
        .map_err(anyhow::Error::from)
        .and_then(|r| r.decode().map_err(anyhow::Error::from));
    let _ = std::fs::remove_file(&tmp);
    let img = img?;

    // Original dimensions; fall back to the reduced ones.
    let (mut w, mut h) = (img.width(), img.height());
    if let Ok(q) = Command::new("/usr/bin/sips")
        .args(["-g", "pixelWidth", "-g", "pixelHeight"])
        .arg(path)
        .output()
    {
        let text = String::from_utf8_lossy(&q.stdout);
        let grab = |key: &str| -> Option<u32> {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key))
                .and_then(|v| v.trim().parse().ok())
        };
        if let (Some(ow), Some(oh)) = (grab("pixelWidth:"), grab("pixelHeight:")) {
            // sips reports stored size; match it to the displayed orientation.
            let landscape = img.width() >= img.height();
            if (ow >= oh) == landscape {
                w = ow;
                h = oh;
            } else {
                w = oh;
                h = ow;
            }
        }
    }
    Ok((img, w, h))
}

fn read_exif(path: &Path) -> Option<exif::Exif> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    exif::Reader::new().read_from_container(&mut reader).ok()
}

/// Every readable EXIF field of the main image, as (name, value). Empty when there is none.
pub fn exif_fields(path: &Path) -> Vec<(String, String)> {
    let Some(exif) = read_exif(path) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for f in exif.fields().filter(|f| f.ifd_num == exif::In::PRIMARY) {
        // Opaque blobs are noise in a list meant for people.
        if matches!(
            f.value,
            exif::Value::Undefined(..) | exif::Value::Unknown(..)
        ) && !matches!(f.tag, exif::Tag::ExifVersion | exif::Tag::FlashpixVersion)
        {
            continue;
        }
        let mut value = f.display_value().with_unit(&exif).to_string();
        if value.is_empty() {
            continue;
        }
        if value.chars().count() > 200 {
            value = value.chars().take(200).collect::<String>() + "…";
        }
        let name = f.tag.to_string();
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, value));
        }
    }
    out
}

fn exif_u32(path: &Path, tag: exif::Tag) -> Option<u32> {
    read_exif(path)?
        .get_field(tag, exif::In::PRIMARY)?
        .value
        .get_uint(0)
}

/// Capture time as seconds since the epoch, reading the EXIF local time as if it were UTC.
/// Good enough for "taken within N seconds of each other".
fn exif_taken_at(path: &Path) -> Option<i64> {
    let exif = read_exif(path)?;
    let field = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
    let exif::Value::Ascii(ref v) = field.value else {
        return None;
    };
    let dt = exif::DateTime::from_ascii(v.first()?).ok()?;
    if dt.year < 1971 {
        return None;
    }
    let days = days_from_civil(dt.year as i64, dt.month as i64, dt.day as i64);
    Some(days * 86400 + dt.hour as i64 * 3600 + dt.minute as i64 * 60 + dt.second as i64)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_days() {
        assert_eq!(super::days_from_civil(1970, 1, 1), 0);
        assert_eq!(super::days_from_civil(2024, 3, 1), 19783);
    }
}
