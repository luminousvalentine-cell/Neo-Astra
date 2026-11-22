use base64::Engine;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::error::NopmError;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex(&digest)
}

pub fn verify_integrity_bytes(bytes: &[u8], integrity: &str) -> Result<(), NopmError> {
    if integrity.starts_with("git:") {
        return Ok(());
    }

    if let Some((algorithm, expected)) = integrity.split_once('-')
        && matches!(algorithm, "sha256" | "sha384" | "sha512")
        && !expected.is_empty()
    {
        let actual = sri_digest(bytes, algorithm);
        if actual != expected {
            return Err(mismatch(integrity));
        }
        return Ok(());
    }

    if let Some((algorithm, expected)) = integrity.split_once(':')
        && matches!(algorithm, "sha256" | "sha384" | "sha512")
        && !expected.is_empty()
    {
        let actual = hex_digest(bytes, algorithm);
        if actual != expected.to_lowercase() {
            return Err(mismatch(integrity));
        }
        return Ok(());
    }

    Err(NopmError::with_path(
        "NP0011",
        format!("Unsupported integrity format '{integrity}'."),
        integrity,
    ))
}

fn mismatch(integrity: &str) -> NopmError {
    NopmError::with_path(
        "NP0011",
        format!("Integrity check failed for '{integrity}'."),
        integrity,
    )
}

fn sri_digest(bytes: &[u8], algorithm: &str) -> String {
    match algorithm {
        "sha256" => {
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(bytes).as_slice())
        }
        "sha384" => {
            base64::engine::general_purpose::STANDARD.encode(Sha384::digest(bytes).as_slice())
        }
        _ => base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes).as_slice()),
    }
}

fn hex_digest(bytes: &[u8], algorithm: &str) -> String {
    match algorithm {
        "sha256" => hex(&Sha256::digest(bytes)),
        "sha384" => hex(&Sha384::digest(bytes)),
        _ => hex(&Sha512::digest(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
