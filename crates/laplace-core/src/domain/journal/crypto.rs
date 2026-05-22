// SPDX-License-Identifier: Apache-2.0
//! ARD AES-GCM symmetric encryption.
//!
//! Key source: `LAPLACE_ARD_KEY` environment variable (64 hex chars = 32 bytes).
//! Payment gating (P3-12) wraps this module externally and does not modify it.

use super::ArdReport;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand08::RngCore;

/// Encrypted ARD binary file magic.
pub const LARD_MAGIC: &[u8; 4] = b"LARD";
/// AES-GCM nonce length.
pub const NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length.
pub const TAG_LEN: usize = 16;

/// AES-GCM codec for ARD reports.
#[derive(Debug)]
pub struct ArdCrypto {
    /// 32-byte AES-256-GCM key.
    key: [u8; 32],
}

impl ArdCrypto {
    /// Loads a 32-byte key from `LAPLACE_ARD_KEY` as 64 hex characters.
    pub fn from_env() -> Result<Self, ArdCryptoError> {
        let key_hex = std::env::var("LAPLACE_ARD_KEY").map_err(|_| ArdCryptoError::MissingKey)?;
        let key = decode_hex_key(&key_hex)?;
        Ok(Self { key })
    }

    /// Creates a codec from raw key bytes.
    pub fn from_key_bytes(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Encrypts an ARD report into `[magic: 4][nonce: 12][ciphertext+tag: N]`.
    pub fn encrypt(&self, report: &ArdReport) -> Result<Vec<u8>, ArdCryptoError> {
        let json = report
            .to_json()
            .map_err(|e| ArdCryptoError::SerdeError(e.to_string()))?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand08::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, json.as_bytes())
            .map_err(|e| ArdCryptoError::EncryptFailed(e.to_string()))?;

        let mut output = Vec::with_capacity(LARD_MAGIC.len() + NONCE_LEN + ciphertext.len());
        output.extend_from_slice(LARD_MAGIC);
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// Decrypts an encrypted ARD byte sequence back into an [`ArdReport`].
    pub fn decrypt(&self, data: &[u8]) -> Result<ArdReport, ArdCryptoError> {
        if data.len() < LARD_MAGIC.len() + NONCE_LEN + TAG_LEN {
            return Err(ArdCryptoError::DecryptFailed);
        }
        if &data[..LARD_MAGIC.len()] != LARD_MAGIC {
            return Err(ArdCryptoError::InvalidMagic);
        }

        let nonce_start = LARD_MAGIC.len();
        let nonce_end = nonce_start + NONCE_LEN;
        let nonce = Nonce::from_slice(&data[nonce_start..nonce_end]);
        let ciphertext = &data[nonce_end..];
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| ArdCryptoError::DecryptFailed)?;
        let json =
            String::from_utf8(plaintext).map_err(|e| ArdCryptoError::SerdeError(e.to_string()))?;

        ArdReport::from_json(&json).map_err(|e| ArdCryptoError::SerdeError(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArdCryptoError {
    #[error("environment variable LAPLACE_ARD_KEY is missing")]
    MissingKey,
    #[error("invalid LAPLACE_ARD_KEY format: {0}")]
    InvalidKey(String),
    #[error("encryption failed: {0}")]
    EncryptFailed(String),
    #[error("decryption failed (key mismatch or tampered data)")]
    DecryptFailed,
    #[error("magic bytes mismatch: not an encrypted ARD file")]
    InvalidMagic,
    #[error("JSON serialization error: {0}")]
    SerdeError(String),
}

fn decode_hex_key(input: &str) -> Result<[u8; 32], ArdCryptoError> {
    if input.len() != 64 {
        return Err(ArdCryptoError::InvalidKey(
            "expected 64 hex characters".to_string(),
        ));
    }

    let mut key = [0u8; 32];
    for idx in 0..32 {
        let chunk = &input[idx * 2..idx * 2 + 2];
        key[idx] = u8::from_str_radix(chunk, 16).map_err(|_| {
            ArdCryptoError::InvalidKey(format!("non-hex byte at offset {}", idx * 2))
        })?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::journal::{ArdHeader, ForensicFrame};

    fn test_report() -> ArdReport {
        ArdReport::new(
            ArdHeader::new(42, "target", "snapshot"),
            vec![ForensicFrame::new(
                0,
                "t0",
                "lock",
                "r0",
                "ok",
                vec!["meta".into()],
            )],
        )
    }

    #[test]
    fn test_ard_encrypt_decrypt_roundtrip() {
        let crypto = ArdCrypto::from_key_bytes([7; 32]);
        let report = test_report();

        let encrypted = crypto.encrypt(&report).unwrap();
        let decrypted = crypto.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, report);
    }

    #[test]
    fn test_ard_decrypt_fails_on_tampered_data() {
        let crypto = ArdCrypto::from_key_bytes([7; 32]);
        let report = test_report();
        let mut encrypted = crypto.encrypt(&report).unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0x01;

        assert!(matches!(
            crypto.decrypt(&encrypted),
            Err(ArdCryptoError::DecryptFailed)
        ));
    }

    #[test]
    fn test_ard_decrypt_fails_on_wrong_magic() {
        let crypto = ArdCrypto::from_key_bytes([7; 32]);
        let report = test_report();
        let mut encrypted = crypto.encrypt(&report).unwrap();
        encrypted[0] = b'X';

        assert!(matches!(
            crypto.decrypt(&encrypted),
            Err(ArdCryptoError::InvalidMagic)
        ));
    }

    #[test]
    fn test_missing_env_key_returns_error() {
        std::env::remove_var("LAPLACE_ARD_KEY");

        assert!(matches!(
            ArdCrypto::from_env(),
            Err(ArdCryptoError::MissingKey)
        ));
    }

    #[test]
    fn test_nonce_is_unique_per_encrypt() {
        let crypto = ArdCrypto::from_key_bytes([7; 32]);
        let report = test_report();

        let first = crypto.encrypt(&report).unwrap();
        let second = crypto.encrypt(&report).unwrap();

        assert_ne!(first, second);
        assert_eq!(&first[..LARD_MAGIC.len()], LARD_MAGIC);
        assert_eq!(&second[..LARD_MAGIC.len()], LARD_MAGIC);
    }
}
