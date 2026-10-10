//! Table-of-contents encryption: AES-256 in CFB-128 mode with a zero IV.
//! CFB is a stream mode, so the ciphertext has exactly the plaintext length.

use aes::Aes256;
use cfb_mode::{BufDecryptor, BufEncryptor};
use cfb_mode::cipher::KeyIvInit;
use md5::{Digest, Md5};

const PSARC_KEY: [u8; 32] = [
    0xC5, 0x3D, 0xB2, 0x38, 0x70, 0xA1, 0xA2, 0xF7, 0x1C, 0xAE, 0x64, 0x06, 0x1F, 0xDD, 0x0E, 0x11,
    0x57, 0x30, 0x9D, 0xC8, 0x52, 0x04, 0xD4, 0xC5, 0xBF, 0xDF, 0x25, 0x09, 0x0D, 0xF2, 0x57, 0x2C,
];

pub(crate) fn encrypt_toc(data: &mut [u8]) {
    BufEncryptor::<Aes256>::new(&PSARC_KEY.into(), &[0u8; 16].into()).encrypt(data);
}

pub(crate) fn decrypt_toc(data: &mut [u8]) {
    BufDecryptor::<Aes256>::new(&PSARC_KEY.into(), &[0u8; 16].into()).decrypt(data);
}

/// MD5 of an entry name; the unnamed manifest entry uses all zeros.
pub(crate) fn name_digest(name: &str) -> [u8; 16] {
    if name.is_empty() {
        [0; 16]
    } else {
        Md5::digest(name.as_bytes()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_odd_length() {
        let plain: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut buf = plain.clone();
        encrypt_toc(&mut buf);
        assert_ne!(buf, plain);
        decrypt_toc(&mut buf);
        assert_eq!(buf, plain);
    }

    #[test]
    fn manifest_digest_is_zero() {
        assert_eq!(name_digest(""), [0; 16]);
        assert_ne!(name_digest("appid.appid"), [0; 16]);
    }
}
