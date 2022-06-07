use std::io;
use std::io::Write;

use anyhow::{Context, Result};
use brotli2::write::BrotliEncoder;
use encoding::DecoderTrap;
use encoding::label::encoding_from_whatwg_label;

pub fn compress_body(/*new_headers: &mut HeaderMap, */body_bytes: &Vec<u8>) -> Result<Vec<u8>> {
    let buf = Vec::new();
    let mut compressor = BrotliEncoder::new(buf, 11);
    let _ = compressor
        .write(body_bytes.as_slice())
        .context("compress error")?;
    let result = compressor.finish()?;
    Ok(result)
}

pub fn to_utf8(orig: &[u8], charset: &str) -> Result<Vec<u8>> {
    let e1 = encoding_from_whatwg_label(charset).context("encoding error")?;
    let s = e1.decode(orig, DecoderTrap::Strict).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decode error: {}", charset),
        )
    })?;
    Ok(s.into_bytes())
}
