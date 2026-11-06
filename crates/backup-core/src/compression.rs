use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use crate::{Error, Result};

/// Decode a zstd file into `dst`; used for legacy JSON backups. Blocking.
pub fn decompress_file(src: &Path, dst: &Path) -> Result<()> {
    let input = File::open(src).map_err(|e| Error::io(format!("Could not open {src:?}"), e))?;
    let output =
        File::create(dst).map_err(|e| Error::io(format!("Could not create {dst:?}"), e))?;
    let mut writer = BufWriter::new(output);

    zstd::stream::copy_decode(BufReader::new(input), &mut writer)
        .map_err(|e| Error::io("Could not decompress legacy backup", e))?;
    writer
        .flush()
        .map_err(|e| Error::io("Could not write decompressed backup", e))
}

#[cfg(test)]
mod tests {
    use super::decompress_file;

    #[test]
    fn test_decompress_file_restores_encoded_input() {
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("in.zst"), dir.path().join("out.json"));
        let input = b"{\"materials\": []}".repeat(100);
        std::fs::write(&src, zstd::encode_all(input.as_slice(), 3).unwrap()).unwrap();

        decompress_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), input);
    }

    #[test]
    fn test_decompress_file_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("in.zst"), dir.path().join("out.json"));
        std::fs::write(&src, b"not zstd").unwrap();

        assert!(decompress_file(&src, &dst).is_err());
    }
}
