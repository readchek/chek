// Adapted from Keiyoushi Comix 1.6.54 Descrambler.kt (Apache-2.0).
use chek_extension::{Error, ImageData, http::Response, images};

fn xorshift(mut state: u32) -> u32 {
    state ^= state << 13;
    state ^= state >> 17;
    state ^ (state << 5)
}

fn media(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 {
        return None;
    }
    if bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG") {
        Some("image/png")
    } else {
        None
    }
}

fn decode(bytes: &[u8], mut state: u32, length: usize, mode: u8) -> Vec<u8> {
    let mut result = bytes.to_vec();
    for byte in result.iter_mut().take(length) {
        state = if mode == 0 {
            state.wrapping_mul(1000005).wrapping_add(1234567891)
        } else {
            xorshift(state)
        };
        *byte ^= if mode == 1 { state } else { state >> 24 } as u8;
    }
    result
}

pub fn transform(response: Response) -> Result<Option<ImageData>, Error> {
    let header = |name: &str| {
        response
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    let seed = header("x-enc-seed")
        .and_then(|v| v.parse::<i64>().ok())
        .map(|v| v as u32);
    let length = header("x-enc-len").and_then(|v| v.parse::<usize>().ok());
    let grid_seed = header("x-scramble-seed")
        .and_then(|v| v.parse::<i64>().ok())
        .map(|v| v as u32);
    let grid_algo = header("x-scramble-algo");
    let grid = header("x-scramble-grid") == Some("5x5")
        && matches!(grid_algo, None | Some("1" | "2" | "3"))
        && grid_seed.is_some_and(|seed| seed != 0);
    let xor = seed.is_some_and(|seed| seed != 0) && length.is_some();
    if !xor && !grid {
        return Ok(None);
    }
    let bytes = if let (Some(seed), Some(length)) = (seed.filter(|v| *v != 0), length) {
        if header("x-enc-algo") == Some("2") {
            let first = decode(&response.body, seed | 1, length, 1);
            if media(&first).is_some() {
                first
            } else {
                [(seed, 1), (seed | 1, 2), (seed, 0)]
                    .into_iter()
                    .map(|(seed, mode)| decode(&response.body, seed, length, mode))
                    .find(|bytes| media(bytes).is_some())
                    .unwrap_or(first)
            }
        } else {
            decode(&response.body, seed, length, 0)
        }
    } else {
        response.body.clone()
    };
    if grid {
        let hash = match header("x-scramble-hash").map(str::trim) {
            Some("03632") => 58414,
            Some("02900") => 117532,
            _ => 0,
        };
        let mut state = grid_seed.unwrap_or_default() ^ hash;
        if grid_algo == Some("3") {
            state |= 1;
        }
        let mut order: Vec<u32> = (0..25).collect();
        for i in (1..25).rev() {
            state = if grid_algo == Some("3") {
                xorshift(state)
            } else {
                state.wrapping_mul(1664525).wrapping_add(1013904223)
            };
            order.swap(i, state as usize % (i + 1));
        }
        let mut inverse = vec![0; 25];
        for (index, tile) in order.into_iter().enumerate() {
            inverse[tile as usize] = index as u32;
        }
        return Ok(Some(ImageData {
            content_type: "image/png".into(),
            body: images::rearrange(&bytes, 5, 5, &inverse).map_err(Error::Other)?,
        }));
    }
    let content_type = media(&bytes)
        .ok_or_else(|| Error::Parse("Comix image decoding did not produce an image".into()))?;
    Ok(Some(ImageData {
        content_type: content_type.into(),
        body: bytes,
    }))
}
