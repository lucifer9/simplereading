use std::io::{self, Write};

use anyhow::{Context, Result};
use encoding::{label::encoding_from_whatwg_label, DecoderTrap};
use futures_util::{SinkExt, StreamExt};
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

pub fn compress_body(/*new_headers: &mut HeaderMap, */ body_bytes: &Vec<u8>,) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut compressor = brotli::CompressorWriter::new(&mut buf, body_bytes.len(), 11, 22);
        compressor.write_all(body_bytes)?;
    }
    Ok(buf)
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

pub async fn get_mp3(ssml: &str) -> Result<Vec<u8>> {
    let dt_fmt = format_description::parse(DATE_FORMAT_STR)?;
    let uuid = Uuid::new_v4().as_simple().to_string().to_uppercase();
    let mut url = String::from(ENDPOINT2);
    url.push_str("?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4");
    url.push_str(format!("&X-ConnectionId={}", &uuid).as_str());

    let mut req = url.into_client_request()?;
    req.headers_mut().append(
        "Origin",
        HeaderValue::from_static("chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"),
    );
    req.headers_mut().append(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36 Edg/107.0.1418.62"));
    let (ws, _) = connect_async(req).await.expect("ws connect error");
    let (mut writer, mut reader) = ws.split();
    let mut message_1 = format!(
        "X-Timestamp: {}\r\nContent-Type: application/json; charset=utf-8\r\nPath: speech.config\r\n\r\n",
        OffsetDateTime::now_utc().format(&dt_fmt)?);
    message_1.push_str(PAYLOAD_2);
    writer.send(message_1.into()).await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message_1 send error: {}", e),
        )
    })?;
    let mut message_2=format!("X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}\r\nPath:ssml\r\n\r\n",&uuid,OffsetDateTime::now_utc().format(&dt_fmt)?);
    message_2.push_str(ssml);
    writer.send(message_2.into()).await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message_2 send error: {}", e),
        )
    })?;
    let mut mp3: Vec<u8> = Vec::new();
    let pat = "Path:audio\r\n".as_bytes().to_vec();
    loop {
        let d = reader.next().await.context("reading ws")?;
        if d.is_err() {
            break;
        }
        let data = d?;
        if data.is_text() {
            if data.into_text()?.contains("Path:turn.end") {
                // let mut file = std::fs::File::create("a.mp3")?;
                // file.write_all(mp3.as_slice())?;
                break;
            }
        } else if data.is_binary() {
            let all = data.into_data();
            let index = all
                .windows(pat.len())
                .position(|window| window == pat)
                .context("no Path:audio in binary")?;
            mp3.extend_from_slice(&all[index + pat.len()..]);
        }
    }
    Ok(mp3)
}
