use std::io::Write;

use anyhow::{Context, Result};
use brotli2::write::BrotliEncoder;
use hyper::HeaderMap;

pub fn compress_body(new_headers: &mut HeaderMap, body_bytes: &Vec<u8>) -> Result<Vec<u8>> {
    let buf = Vec::new();
    let mut compressor = BrotliEncoder::new(buf, 11);
    let _ = compressor
        .write(body_bytes.as_slice())
        .context("compress error")?;
    let result = compressor.finish()?;
    new_headers.insert(hyper::header::CONTENT_ENCODING, "br".parse()?);
    new_headers.insert(
        hyper::header::CONTENT_LENGTH,
        result.len().to_string().parse()?,
    );
    Ok(result)
}
