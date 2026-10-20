//! Raster work runs outside the server, so a slow decoder can be stopped.

use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use libvips::{VipsApp, VipsImage, ops};
use serde::Deserialize;
use serde_json::json;

use crate::worker::Worker;

pub(crate) const MAX_BYTES: usize = 8 << 20;
const MAX_PIXELS: i64 = 8_000_000;
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
struct Tiles {
    body: String,
    columns: u32,
    rows: u32,
    order: Vec<u32>,
}

pub(crate) fn rearrange(
    path: &Path,
    body: &[u8],
    columns: u32,
    rows: u32,
    order: &[u32],
) -> Result<Vec<u8>, String> {
    validate(body, columns, rows, order)?;
    let mut command = Command::new(path);
    command.arg("--extension-image-worker");
    let worker = Worker::start(command, TIMEOUT)?;
    worker.send(
        &json!({"body": STANDARD.encode(body), "columns": columns, "rows": rows, "order": order}),
        worker.deadline,
    )?;
    let result = worker.receive(worker.deadline)?;
    if let Some(error) = result.get("error").and_then(|v| v.as_str()) {
        return Err(error.to_owned());
    }
    let encoded = result
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or("image worker returned no image")?;
    let bytes = STANDARD.decode(encoded).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("transformed image exceeds 8 MiB".to_owned());
    }
    Ok(bytes)
}

fn validate(body: &[u8], columns: u32, rows: u32, order: &[u32]) -> Result<(), String> {
    if body.len() > MAX_BYTES {
        return Err("image exceeds 8 MiB".to_owned());
    }
    let tiles = columns
        .checked_mul(rows)
        .filter(|n| *n > 0 && *n <= 256)
        .ok_or("grid must have 1 to 256 tiles")?;
    let mut sorted = order.to_vec();
    sorted.sort_unstable();
    if sorted != (0..tiles).collect::<Vec<_>>() {
        return Err("tile order must be a permutation of the grid".to_owned());
    }
    let supported = body.starts_with(b"\xff\xd8\xff")
        || body.starts_with(b"\x89PNG\r\n\x1a\n")
        || (body.starts_with(b"RIFF") && body.get(8..12) == Some(b"WEBP"));
    if !supported {
        return Err("image must be JPEG, PNG or WebP".to_owned());
    }
    Ok(())
}

/// Internal worker entry point, called by the server before it reads config
/// or starts serving. It accepts one bounded JSON line and then exits.
pub fn image_worker() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take((16 << 20) + 1)
        .read_until(b'\n', &mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > 16 << 20 {
        return Err("image worker input is too large".to_owned());
    }
    let result = serde_json::from_slice::<Tiles>(&input)
        .map_err(|e| e.to_string())
        .and_then(render);
    let result = match result {
        Ok(body) => json!({"body": STANDARD.encode(body)}),
        Err(error) => json!({"error": error}),
    };
    writeln!(std::io::stdout(), "{result}").map_err(|e| e.to_string())
}

fn render(tiles: Tiles) -> Result<Vec<u8>, String> {
    let body = STANDARD.decode(&tiles.body).map_err(|e| e.to_string())?;
    validate(&body, tiles.columns, tiles.rows, &tiles.order)?;
    let app = VipsApp::new("chek-image-worker", false).map_err(|e| e.to_string())?;
    app.concurrency_set(1);
    app.cache_set_max(0);
    let image = VipsImage::new_from_buffer(&body, "").map_err(|e| e.to_string())?;
    if !matches!(
        image.get_format().map_err(|e| e.to_string())?,
        ops::BandFormat::Uchar
    ) {
        return Err("image must use 8-bit channels".to_owned());
    }
    let (width, height) = (image.get_width(), image.get_height());
    if width <= 0
        || height <= 0
        || i64::from(width) * i64::from(height) > MAX_PIXELS
        || image.get_bands() > 4
    {
        return Err("image exceeds 8 million pixels or four bands".to_owned());
    }
    let (columns, rows) = (tiles.columns as i32, tiles.rows as i32);
    let (tile_width, tile_height) = (width / columns, height / rows);
    if tile_width == 0 || tile_height == 0 {
        return Err("grid is larger than the image".to_owned());
    }
    // VipsImage's derived Clone duplicates a raw handle without retaining it.
    let mut output = ops::copy(&image).map_err(|e| e.to_string())?;
    for (destination, source) in tiles.order.iter().enumerate() {
        let source = *source as i32;
        let destination = destination as i32;
        let tile = ops::extract_area(
            &image,
            (source % columns) * tile_width,
            (source / columns) * tile_height,
            tile_width,
            tile_height,
        )
        .map_err(|e| e.to_string())?;
        output = ops::insert(
            &output,
            &tile,
            (destination % columns) * tile_width,
            (destination / columns) * tile_height,
        )
        .map_err(|e| e.to_string())?;
    }
    let bytes = output
        .image_write_to_buffer(".png[strip]")
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("transformed image exceeds 8 MiB".to_owned());
    }
    Ok(bytes)
}
