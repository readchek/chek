//! Cover thumbnails, made with libvips.

use std::path::Path;
use std::sync::OnceLock;

use libvips::{VipsApp, VipsImage, ops};

use crate::{Format, FormatError, PageEntry, epub, read_page};

/// Covers fit inside this box, keeping their aspect ratio.
const COVER_WIDTH: i32 = 400;
const COVER_HEIGHT: i32 = 600;
const COVER_QUALITY: i32 = 82;

/// Makes a JPEG thumbnail of a publication's cover. Returns `None` when the
/// publication has no cover to show, such as an EPUB that doesn't mark one.
pub fn cover_thumbnail(
    path: &Path,
    format: Format,
    pages: &[PageEntry],
) -> Result<Option<Vec<u8>>, FormatError> {
    vips()?;
    // Only the operations' required arguments are passed. This crate's
    // `_with_opts` calls set every option of the newest libvips, which fails on
    // older ones, so sizing is computed here and saving takes a suffix string.
    let source = match format {
        Format::Cbz | Format::Cbt | Format::Cbr | Format::Cb7 => {
            let Some(first) = pages.first() else {
                return Ok(None);
            };
            Source::Bytes(read_page(path, first)?)
        }
        Format::Epub => match epub::cover_image(path)? {
            Some(bytes) => Source::Bytes(bytes),
            None => return Ok(None),
        },
        // libvips renders the first page of a PDF through poppler.
        Format::Pdf => match path.to_str() {
            Some(path) => Source::File(path),
            None => return Ok(None),
        },
    };
    // libvips decodes lazily, reading the source while the JPEG is written, so
    // `source` has to outlive the save below.
    let image = match &source {
        Source::Bytes(bytes) => {
            let header = VipsImage::new_from_buffer(bytes, "").map_err(vips_error)?;
            ops::thumbnail_buffer(bytes, target_size(&header))
        }
        Source::File(path) => {
            let header = VipsImage::new_from_file(path).map_err(vips_error)?;
            ops::thumbnail(path, target_size(&header))
        }
    }
    .map_err(vips_error)?;
    let jpeg = image
        .image_write_to_buffer(&format!(".jpg[Q={COVER_QUALITY},strip,optimize-coding]"))
        .map_err(vips_error)?;
    drop(image);
    drop(source);
    Ok(Some(jpeg))
}

/// Makes a JPEG cover thumbnail from an image, such as a thumbnail another
/// server made.
pub fn cover_from_image(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    vips()?;
    let header = VipsImage::new_from_buffer(bytes, "").map_err(vips_error)?;
    let image = ops::thumbnail_buffer(bytes, target_size(&header)).map_err(vips_error)?;
    image
        .image_write_to_buffer(&format!(".jpg[Q={COVER_QUALITY},strip,optimize-coding]"))
        .map_err(vips_error)
}

enum Source<'a> {
    Bytes(Vec<u8>),
    File(&'a str),
}

/// The size to pass to libvips' thumbnail, which fits the image in a square of
/// that size. Using the long side of the scaled image fits it in the cover box
/// instead, without ever enlarging it.
fn target_size(image: &VipsImage) -> i32 {
    let (width, height) = (
        f64::from(image.get_width().max(1)),
        f64::from(image.get_height().max(1)),
    );
    let scale = (f64::from(COVER_WIDTH) / width)
        .min(f64::from(COVER_HEIGHT) / height)
        .min(1.0);
    ((width.max(height) * scale).round() as i32).max(1)
}

/// Starts libvips once per process. Each call runs on a single libvips
/// thread, because scans already run one file per worker and the worker count
/// is what keeps scans below the core count.
pub(crate) fn vips() -> Result<&'static VipsApp, FormatError> {
    static VIPS: OnceLock<Result<VipsApp, String>> = OnceLock::new();
    VIPS.get_or_init(|| {
        let app = VipsApp::new("chek", false).map_err(|e| e.to_string())?;
        app.concurrency_set(1);
        // Thumbnails are made once per file; caching operations only holds memory.
        app.cache_set_max(0);
        Ok(app)
    })
    .as_ref()
    .map_err(|e| FormatError::Image(format!("libvips failed to start: {e}")))
}

pub(crate) fn vips_error(e: libvips::error::Error) -> FormatError {
    let detail = vips()
        .ok()
        .and_then(|app| {
            let detail = app.error_buffer().ok().map(|b| b.trim().to_owned());
            app.error_clear();
            detail
        })
        .filter(|d| !d.is_empty());
    FormatError::Image(detail.map_or_else(|| e.to_string(), |d| format!("{e}: {d}")))
}
