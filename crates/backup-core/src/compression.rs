use std::io::{BufReader, BufWriter, Cursor, Write};

use zstd::stream::{copy_decode, copy_encode};

use crate::{Errors, Res};

// 3Mb, it should be adjusted for raw db data size.
const BUF_SIZE: usize = 1024 * 1024 * 3;

pub fn compress(inp: &[u8], level: i32) -> Res<Vec<u8>> {
    let mut out = Vec::with_capacity(inp.len());
    {
        let mut src = BufReader::with_capacity(BUF_SIZE, inp);
        let mut dst = BufWriter::new(&mut out);

        copy_encode(&mut src, &mut dst, level)?;

        dst.flush()?;
    }

    Ok(out)
}

pub fn decompress<T>(src: &T) -> Res<Vec<u8>>
where
    T: AsRef<[u8]>,
{
    let mut cur = Cursor::new(src);
    let mut dst = BufWriter::with_capacity(BUF_SIZE, Vec::with_capacity(BUF_SIZE));

    copy_decode(&mut cur, &mut dst)?;

    dst.flush()?;
    let dst = dst.into_inner().map_err(|e| {
        let msg = format!("Could not decompress file body: {e}");
        Errors::StorageError(msg)
    })?;

    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::{compress, decompress};

    #[test]
    fn test_compress_decompress_roundtrip() {
        let input = b"{\"materials\": []}".repeat(100);

        let compressed = compress(&input, 3).unwrap();
        let restored = decompress(&compressed).unwrap();

        assert!(compressed.len() < input.len());
        assert_eq!(restored, input);
    }
}
