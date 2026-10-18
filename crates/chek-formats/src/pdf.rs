//! PDF metadata from the document information dictionary, read through
//! libvips' poppler loader, which already opens PDFs for covers.

use std::path::Path;

use libvips::VipsImage;

use crate::FormatError;
use crate::cover::{vips, vips_error};
use crate::metadata::{MAX_FIELD_CHARS, Metadata, Role, clean, clean_summary, push_contributor};

/// Reads the title, authors and subject a PDF declares. Titles that are only
/// the name of the file the PDF was made from, as office software writes
/// them, are ignored.
pub(crate) fn metadata(path: &Path) -> Result<Metadata, FormatError> {
    let app = vips()?;
    let Some(path) = path.to_str() else {
        return Ok(Metadata::default());
    };
    // Opening reads only the header; no page is rendered.
    let image = VipsImage::new_from_file(path).map_err(vips_error)?;
    let field = |name: &str| match image.get_as_string(name) {
        Ok(value) => Some(value),
        // A missing field is normal; drop the error it leaves behind.
        Err(_) => {
            app.error_clear();
            None
        }
    };

    let mut metadata = Metadata {
        title: field("pdf-title")
            .and_then(|t| clean(&t, MAX_FIELD_CHARS))
            .filter(|t| !is_file_name(t)),
        summary: field("pdf-subject").and_then(|s| clean_summary(&s)),
        ..Metadata::default()
    };
    // Several authors are usually separated by semicolons.
    for name in field("pdf-author").iter().flat_map(|a| a.split(';')) {
        push_contributor(&mut metadata.contributors, name, Role::Author);
    }
    Ok(metadata)
}

/// Whether a title is a document's file name, such as
/// `Microsoft Word - draft3.docx` or `report.tex`, rather than a real title.
fn is_file_name(title: &str) -> bool {
    const EXTENSIONS: [&str; 11] = [
        "doc", "docx", "odt", "rtf", "txt", "tex", "dvi", "pdf", "indd", "pages", "qxd",
    ];
    let lower = title.to_lowercase();
    lower.starts_with("microsoft word - ")
        || lower
            .rsplit_once('.')
            .is_some_and(|(_, ext)| EXTENSIONS.contains(&ext))
}
