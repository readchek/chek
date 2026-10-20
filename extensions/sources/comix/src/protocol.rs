//! Wire format recovered from matching captured responses and client bundles.
//! Key material belongs to a site build and is supplied by that build at runtime.
use base64::{Engine, engine::general_purpose};
use serde_json::Value;
use sha2::{compress256, digest::generic_array::GenericArray};

const MATERIAL_BYTES: usize = 1088;
const TABLE_BYTES: usize = 1024;
const MAX_ENCODED_BYTES: usize = 1 << 20;

fn validate(material: &[u8]) -> Result<(), String> {
    if material.len() != MATERIAL_BYTES {
        return Err("Comix supplied an unsupported codec table".into());
    }
    Ok(())
}

fn digest(seed: &[u8], input: &[u8]) -> [u8; 32] {
    let mut state = [0; 8];
    for (word, bytes) in state.iter_mut().zip(seed.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    // The supplied states have already processed the first 64-byte block.
    padded.extend_from_slice(&(((64 + input.len()) as u64) * 8).to_be_bytes());
    for block in padded.chunks_exact(64) {
        compress256(&mut state, &[GenericArray::clone_from_slice(block)]);
    }
    let mut bytes = [0; 32];
    for (out, word) in bytes.chunks_exact_mut(4).zip(state) {
        out.copy_from_slice(&word.to_be_bytes());
    }
    bytes
}

pub fn sign(material: &[u8], canonical: &str) -> Result<String, String> {
    validate(material)?;
    if canonical.len() > 16 * 1024 {
        return Err("Comix request parameters are too large".into());
    }
    let inner = digest(&material[..32], canonical.as_bytes());
    let outer = digest(&material[32..64], &inner);
    Ok(general_purpose::URL_SAFE_NO_PAD.encode(outer)[..16].into())
}

pub fn decode(material: &[u8], body: &str) -> Result<Value, String> {
    validate(material)?;
    if body.len() > MAX_ENCODED_BYTES {
        return Err("Comix encoded response is too large".into());
    }
    let envelope: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let encoded = envelope["e"]
        .as_str()
        .ok_or("Comix response has no encoded payload")?;
    let mut bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    if bytes.len() < 3 {
        return Err("Comix encoded response is too short".into());
    }
    let rotation = usize::from(u16::from_be_bytes([bytes[0], bytes[1]])) % TABLE_BYTES;
    for (index, byte) in bytes[2..].iter_mut().enumerate() {
        *byte ^= material[64 + (rotation + index) % TABLE_BYTES];
    }
    serde_json::from_slice(&bytes[2..]).map_err(|e| e.to_string())
}
