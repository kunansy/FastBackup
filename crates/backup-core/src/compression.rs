use crate::{Error, Result};

/// Decode a zstd frame; used for legacy JSON backups.
pub fn decompress(src: &[u8]) -> Result<Vec<u8>> {
    zstd::decode_all(src).map_err(|e| Error::io("Could not decompress file body", e))
}

#[cfg(test)]
mod tests {
    use super::decompress;

    #[test]
    fn test_decompress_restores_encoded_input() {
        let input = b"{\"materials\": []}".repeat(100);
        let compressed = zstd::encode_all(input.as_slice(), 3).unwrap();

        assert_eq!(decompress(&compressed).unwrap(), input);
    }

    #[test]
    fn test_decompress_rejects_garbage() {
        assert!(decompress(b"not zstd").is_err());
    }
}
