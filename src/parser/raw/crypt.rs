//! PDF decryption support (Standard Security Handler, R2-R4).
//!
//! Implements password verification and key derivation per the PDF spec,
//! plus per-object decryption using RC4 or AES-128-CBC.

use md5::{Digest, Md5};
use rc4::{KeyInit, Rc4, StreamCipher};

/// PDF encryption parameters parsed from the /Encrypt dictionary.
#[derive(Debug, Clone)]
pub struct EncryptionParams {
    /// /V — algorithm version (1, 2, or 4 for R2-R4).
    pub version: u32,
    /// /R — Standard security handler revision (2, 3, or 4).
    pub revision: u32,
    /// /Length — encryption key length in bits (default 40).
    pub key_length: u32,
    /// /O — owner password hash (32 bytes for R2-R4).
    pub owner_hash: Vec<u8>,
    /// /U — user password hash (32 bytes for R2-R4).
    pub user_hash: Vec<u8>,
    /// /P — permissions flags.
    pub permissions: i32,
    /// First element of trailer /ID array.
    pub file_id: Vec<u8>,
    /// Whether to use AES (true for R4 with /StmF or /StrF = /AESV2).
    pub use_aes: bool,
}

/// The standard 32-byte padding used in PDF encryption (Table 3.19 in PDF spec).
const PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4B, 0x49, 0x43, 0x28, 0x46, 0x57,
    0x44, 0x28, 0x0C, 0x06, 0x08, 0x0E, 0x02, 0x05, 0x05, 0x01, 0x09, 0x14, 0xF2, 0xE4, 0x97, 0x35,
];

/// Derive the file encryption key from a user password (Algorithm 2, PDF spec).
/// Works for Standard Security Handler R2-R4.
pub fn compute_encryption_key(params: &EncryptionParams, password: &[u8]) -> Vec<u8> {
    let key_len = (params.key_length / 8) as usize;

    // Step a: Pad or truncate password to exactly 32 bytes
    let mut padded = Vec::with_capacity(32);
    let take = password.len().min(32);
    padded.extend_from_slice(&password[..take]);
    if padded.len() < 32 {
        padded.extend_from_slice(&PADDING[..32 - padded.len()]);
    }

    // Steps b-f: MD5(padded || O || P || fileID)
    let mut hasher = Md5::new();
    hasher.update(&padded);
    hasher.update(&params.owner_hash);
    hasher.update(params.permissions.to_le_bytes());
    hasher.update(&params.file_id);

    let mut hash = hasher.finalize().to_vec();

    // Step g: For R >= 3, re-hash 50 times (using only key_len bytes)
    if params.revision >= 3 {
        for _ in 0..50 {
            let mut h = Md5::new();
            h.update(&hash[..key_len]);
            hash = h.finalize().to_vec();
        }
    }

    hash.truncate(key_len);
    hash
}

/// Verify user password and return the encryption key if correct.
/// Algorithm 6 (R2) / Algorithm 7 (R3-R4) from the PDF spec.
pub fn authenticate_user_password(params: &EncryptionParams, password: &[u8]) -> Option<Vec<u8>> {
    let key = compute_encryption_key(params, password);

    if params.revision == 2 {
        // Algorithm 4: RC4-encrypt the 32-byte padding with the key
        let encrypted = rc4_crypt(&key, &PADDING);
        if encrypted[..] == params.user_hash[..32.min(params.user_hash.len())] {
            return Some(key);
        }
    } else if params.revision >= 3 && params.revision <= 4 {
        // Algorithm 5: MD5(padding || fileID), then 20 rounds of RC4
        let mut hasher = Md5::new();
        hasher.update(PADDING);
        hasher.update(&params.file_id);
        let hash = hasher.finalize();

        let mut encrypted = hash.to_vec();
        encrypted = rc4_crypt(&key, &encrypted);

        // 19 additional RC4 passes with XOR-modified keys
        for i in 1..=19u8 {
            let modified_key: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
            encrypted = rc4_crypt(&modified_key, &encrypted);
        }

        // Compare first 16 bytes only (rest is random padding)
        if encrypted.len() >= 16
            && params.user_hash.len() >= 16
            && encrypted[..16] == params.user_hash[..16]
        {
            return Some(key);
        }
    }

    None
}

/// RC4 encrypt/decrypt (symmetric operation).
fn rc4_crypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut cipher = Rc4::new_from_slice(key).expect("RC4 key length 1-256");
    let mut output = data.to_vec();
    cipher.apply_keystream(&mut output);
    output
}

/// Compute per-object decryption key (Algorithm 1 from the PDF spec).
///
/// file_key + obj_num (3 LE bytes) + gen_num (2 LE bytes) [+ "sAlT" for AES]
/// hashed with MD5, truncated to min(file_key.len()+5, 16).
pub fn object_key(file_key: &[u8], obj_num: u32, gen_num: u16, use_aes: bool) -> Vec<u8> {
    let mut hasher = Md5::new();
    hasher.update(file_key);
    hasher.update(&obj_num.to_le_bytes()[..3]);
    hasher.update(&gen_num.to_le_bytes()[..2]);
    if use_aes {
        // AES salt per spec
        hasher.update(b"sAlT");
    }
    let hash = hasher.finalize();
    let key_len = (file_key.len() + 5).min(16);
    hash[..key_len].to_vec()
}

/// Decrypt a byte sequence using RC4.
pub fn decrypt_rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    rc4_crypt(key, data)
}

/// Decrypt a byte sequence using AES-128-CBC.
/// The first 16 bytes of `data` are the IV; the remainder is ciphertext.
pub fn decrypt_aes128(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    use aes::Aes128;
    use cbc::cipher::{block_padding, BlockDecryptMut, KeyIvInit};

    if data.len() < 16 || data.len() % 16 != 0 {
        return None;
    }

    let iv = &data[..16];
    let ciphertext = &data[16..];

    if ciphertext.is_empty() {
        return Some(vec![]);
    }

    type Aes128CbcDec = cbc::Decryptor<Aes128>;

    // Try PKCS7 first
    let mut buf = ciphertext.to_vec();
    let decryptor = Aes128CbcDec::new(key.into(), iv.into());
    if let Ok(plaintext) = decryptor.decrypt_padded_mut::<block_padding::Pkcs7>(&mut buf) {
        return Some(plaintext.to_vec());
    }

    // Fallback: no padding (some PDFs omit PKCS7)
    let mut buf2 = ciphertext.to_vec();
    let decryptor2 = Aes128CbcDec::new(key.into(), iv.into());
    if let Ok(plaintext) = decryptor2.decrypt_padded_mut::<block_padding::NoPadding>(&mut buf2) {
        return Some(plaintext.to_vec());
    }

    None
}
