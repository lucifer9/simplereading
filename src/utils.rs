use std::io::{self, Read};

use anyhow::{Context, Result};
use brotli::CompressorReader;
use encoding::{label::encoding_from_whatwg_label, DecoderTrap};
use futures_util::{SinkExt, StreamExt};
use log::info;
use time::{format_description, OffsetDateTime};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::USER_AGENT;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use uuid::Uuid;

const DATE_FORMAT_STR: &str = "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z";

const ENDPOINT2: &str =
    "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const PAYLOAD_2: &str = r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"false"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#;

// Compress the given body using Brotli
pub fn compress_body(/*new_headers: &mut HeaderMap, */ body: &[u8]) -> Result<Vec<u8>> {
    // Create a Brotli encoder with default initialization parameters
    let mut compressor = CompressorReader::new(body, 4096, 11, 22);
    let mut compressed = Vec::new();
    // Compress the content
    compressor.read_to_end(&mut compressed)?;
    Ok(compressed)
}

// Convert the given bytes to UTF-8 using the specified character set
pub fn to_utf8(orig: &[u8], charset: &str) -> Result<String> {
    let e1 = encoding_from_whatwg_label(charset).context("encoding error")?;
    let s = e1.decode(orig, DecoderTrap::Strict).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decode error: {charset}"),
        )
    })?;
    Ok(s)
}

// Send a request to the speech service and return the resulting MP3 audio data
pub async fn get_mp3(ssml: &str) -> Result<Vec<u8>> {
    // Define the timestamp format for the X-Timestamp header
    let dt_fmt = format_description::parse(DATE_FORMAT_STR)?;
    // Generate a unique identifier for the request

    let uuid = Uuid::new_v4().as_simple().to_string().to_uppercase();

    // Construct the WebSocket URL with the TrustedClientToken and X-ConnectionId parameters
    let mut url = String::from(ENDPOINT2);
    url.push_str("?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4");
    url.push_str(format!("&X-ConnectionId={}", &uuid).as_str());
    info!("mp3 url: {}", &url);
    // Convert the URL into a WebSocket request
    let mut req = url.into_client_request()?;
    req.headers_mut().append(
        "Origin",
        HeaderValue::from_static("chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"),
    );
    req.headers_mut().append(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/113.0.0.0 Safari/537.36 Edg/112.0.1722.68"));

    // Send the WebSocket request and split the resulting stream into a writer and a reader
    let (ws, _) = connect_async(req).await.expect("ws connect error");
    // Split the WebSocket into a writer and reader
    let (mut writer, mut reader) = ws.split();
    // Send the first message
    let mut message_1 = format!(
        "X-Timestamp: {}\r\nContent-Type: application/json; charset=utf-8\r\nPath: speech.config\r\n\r\n",
        OffsetDateTime::now_utc().format(&dt_fmt)?);
    message_1.push_str(PAYLOAD_2);

    writer.send(message_1.into()).await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message_1 send error: {e}"),
        )
    })?;
    // Send the second message
    let mut message_2=format!("X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}\r\nPath:ssml\r\n\r\n",&uuid,OffsetDateTime::now_utc().format(&dt_fmt)?);
    message_2.push_str(ssml);
    writer.send(message_2.into()).await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message_2 send error: {e}"),
        )
    })?;

    // Receive the audio data
    let mut mp3: Vec<u8> = Vec::new();
    let pat = "Path:audio\r\n".as_bytes().to_vec();
    loop {
        let d = reader.next().await.context("reading ws")?;
        if let Ok(data) = d {
            if data.is_binary() {
                let all = data.into_data();
                if let Some(index) = all.windows(pat.len()).position(|window| window == pat) {
                    mp3.extend_from_slice(&all[index + pat.len()..]);
                }
            } else if data.is_text() && data.into_text()?.contains("Path:turn.end") {
                // End of audio data
                break;
            }
        }
        // if d.is_err() {
        //     break;
        // }
        // let data = d?;
        // if data.is_text() {
        //     if data.into_text()?.contains("Path:turn.end") {
        //         // let mut file = std::fs::File::create("a.mp3")?;
        //         // file.write_all(mp3.as_slice())?;
        //         break;
        //     }
        // } else if data.is_binary() {
        //     let all = data.into_data();
        //     let index = all
        //         .windows(pat.len())
        //         .position(|window| window == pat)
        //         .context("no Path:audio in binary")?;
        //     mp3.extend_from_slice(&all[index + pat.len()..]);
        // }
    }
    // Return the audio data
    Ok(mp3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_mp3() -> Result<()> {
        let ssml = r#"
            <speak version="1.0" xmlns="https://www.w3.org/2001/10/synthesis" xml:lang="en-US">
                <voice name="en-US-AriaNeural">
                    Hello world
                </voice>
            </speak>
        "#;
        let mp3 = get_mp3(ssml).await?;
        assert!(!mp3.is_empty());
        Ok(())
    }
    #[test]
    fn test_to_utf8() {
        let orig = b"Hello, world!";
        let charset = "utf-8";
        let result = to_utf8(orig, charset).unwrap();
        assert_eq!(result, String::from("Hello, world!"));

        let orig = b"\xc4\xe3\xba\xc3\xa3\xac\xca\xc0\xbd\xe7\xa3\xa1";
        let charset = "gb18030";
        let result = to_utf8(orig, charset).unwrap();
        assert_eq!(result, "你好，世界！");

        // Test with invalid charset
        let orig = b"\xc3\xa9";
        let charset = "invalid-charset";
        assert!(to_utf8(orig, charset).is_err());
    }

    #[test]
    fn test_compress_body() {
        let body_bytes = b"Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!".to_vec();
        let result = compress_body(&body_bytes).unwrap();
        assert!(result.len() < body_bytes.len());
    }
}
