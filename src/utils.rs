use std::io::Write;
use std::io::{self};

use anyhow::{Context, Result};
use brotli2::write::BrotliEncoder;
use chrono::prelude::*;
use encoding::label::encoding_from_whatwg_label;
use encoding::DecoderTrap;
use futures_util::{SinkExt, StreamExt};

use tokio_tungstenite::connect_async;
use url::Url;
use uuid::Uuid;

const ENDPOINT2: &str = "wss://eastus.api.speech.microsoft.com/cognitiveservices/websocket/v1";
const PAYLOAD_1: &str = r#"{"context":{"system":{"name":"SpeechSDK","version":"1.19.0","build":"JavaScript","lang":"JavaScript","os":{"platform":"Browser/Linux x86_64","name":"Mozilla/5.0 (X11; Linux x86_64; rv:78.0) Gecko/20100101 Firefox/78.0","version":"5.0 (X11)"}}}}"#;
const PAYLOAD_2: &str = r#"{"synthesis":{"audio":{"metadataOptions":{"sentenceBoundaryEnabled":false,"wordBoundaryEnabled":false},"outputFormat":"audio-16khz-32kbitrate-mono-mp3"}}}"#;

pub fn compress_body(/*new_headers: &mut HeaderMap, */ body_bytes: &Vec<u8>,) -> Result<Vec<u8>> {
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

pub async fn get_mp3(ssml: &str) -> Result<Vec<u8>> {
    let uuid = Uuid::new_v4().as_simple().to_string().to_uppercase();
    let mut url = String::from(ENDPOINT2);
    url.push_str("?TricType=AzureDemo");
    url.push_str(format!("&X-ConnectionId={}", &uuid).as_str());
    let (ws, _) = connect_async(Url::parse(&url)?)
        .await
        .expect("ws connect error");
    let (mut writer, mut reader) = ws.split();
    let message_1=format!("Path : speech.config\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/json\r\n\r\n{}",&uuid,Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),PAYLOAD_1);
    writer.send(message_1.into()).await?;
    let message_2=format!("Path : synthesis.context\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/json\r\n\r\n{}",&uuid,Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),PAYLOAD_2);
    writer.send(message_2.into()).await?;
    let message_3=format!("Path: ssml\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/ssml+xml\r\n\r\n{}",&uuid,Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),ssml);
    writer.send(message_3.into()).await?;
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
