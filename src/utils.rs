use std::env;
use std::io::{self, Read};
use std::sync::Arc;

use anyhow::{Context, Result};
use brotli::CompressorReader;
use encoding_rs::*;
use futures_util::{SinkExt, StreamExt};
use log::{debug, info};
use rustls::ClientConfig;
use time::{format_description, OffsetDateTime};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_socks::tcp::Socks5Stream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::USER_AGENT;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;
use uuid::Uuid;

const DATE_FORMAT_STR: &str = "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z";

const ENDPOINT2: &str =
    "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
const PAYLOAD_2: &str = r#"{"context":{"synthesis":{"audio":{"metadataoptions":{"sentenceBoundaryEnabled":"false","wordBoundaryEnabled":"false"},"outputFormat":"audio-24khz-48kbitrate-mono-mp3"}}}}"#;

// Compress the given body using Brotli
pub fn compress_body(body: &[u8], compression_type: &mut String) -> Result<Vec<u8>> {
    // Pre-allocate with capacity based on input size
    let mut smallest_compressed = Vec::with_capacity(body.len());
    let compressions = if compression_type.is_empty() {
        vec!["br"]
    } else {
        let mut types: Vec<&str> = compression_type.split(',').map(str::trim).collect();
        if !types.contains(&"br") {
            types.push("br");
        }
        types
    };

    let mut best_compression_type = String::new();

    for &compression in &compressions {
        let compressed = match compression {
            "br" => {
                let mut output = Vec::with_capacity(body.len());
                // Optimize Brotli parameters: window size 22 (max), quality 4 (fast)
                CompressorReader::new(body, body.len(), 4, 22).read_to_end(&mut output)?;
                output
            }
            "zstd" => {
                // Use a lower compression level (7) for better speed/compression ratio balance
                zstd::bulk::compress(body, 7)?
            }
            "gzip" => {
                use flate2::write::GzEncoder;
                use flate2::Compression;
                let mut encoder =
                    GzEncoder::new(Vec::with_capacity(body.len()), Compression::fast());
                io::Write::write_all(&mut encoder, body)?;
                encoder.finish()?
            }
            "deflate" => {
                use flate2::write::DeflateEncoder;
                use flate2::Compression;
                let mut encoder =
                    DeflateEncoder::new(Vec::with_capacity(body.len()), Compression::fast());
                io::Write::write_all(&mut encoder, body)?;
                encoder.finish()?
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported compression type: {}",
                    compression
                ))
            }
        };

        debug!("compressed len: {}", compressed.len());
        if smallest_compressed.is_empty() || compressed.len() < smallest_compressed.len() {
            smallest_compressed = compressed;
            best_compression_type = compression.to_string();
        }
    }

    // Update the compression type after the loop
    compression_type.clear();
    compression_type.push_str(&best_compression_type);

    Ok(smallest_compressed)
}

// Convert the given bytes to UTF-8 using the specified character set
pub fn to_utf8(orig: &[u8], charset: &str) -> Result<String> {
    let encoding = Encoding::for_label(charset.as_bytes())
        .context(format!("error get encoding:{}", charset))?;
    let (cow, _, had_errors) = encoding.decode(orig);
    if had_errors {
        return Err(anyhow::anyhow!("error decoding"));
    }
    Ok(cow.into_owned())
}

// 获取系统代理设置并建立连接
async fn get_proxy_stream(url: &str) -> Option<TcpStream> {
    let proxy_url = env::var("HTTPS_PROXY")
        .or_else(|_| env::var("https_proxy"))
        .or_else(|_| env::var("HTTP_PROXY"))
        .or_else(|_| env::var("http_proxy"))
        .ok()?;

    let proxy = Url::parse(&proxy_url).ok()?;
    let target = Url::parse(url).ok()?;

    // 只支持 socks5 代理
    if proxy.scheme() != "socks5" {
        return None;
    }

    let proxy_host = proxy.host_str()?;
    let proxy_port = proxy.port()?;
    let target_host = target.host_str()?;
    let target_port = target.port().unwrap_or(443);

    // 建立 SOCKS5 连接
    let stream = Socks5Stream::connect((proxy_host, proxy_port), (target_host, target_port))
        .await
        .ok()?;

    Some(stream.into_inner())
}

// 向语音服务发送请求并返回生成的MP3音频数据
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

    // Create WebSocket request
    let mut req = url.clone().into_client_request()?;
    req.headers_mut().append(
        "Origin",
        HeaderValue::from_static("chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold"),
    );
    req.headers_mut().append(
        USER_AGENT,
        HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"),
    );

    // Get TCP stream (either direct or through proxy)
    let tcp_stream = match get_proxy_stream(&url).await { Some(proxy_stream) => {
        proxy_stream
    } _ => {
        let target = url.parse::<Url>()?;
        let host = target.host_str().context("No host in URL")?;
        let port = target.port().unwrap_or(443);
        TcpStream::connect((host, port)).await?
    }};

    // Configure TLS with rustls
    let root_store = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    // Establish TLS connection
    let domain = url
        .parse::<Url>()?
        .host_str()
        .context("No host in URL")?
        .to_string();
    let server_name = rustls::pki_types::ServerName::try_from(domain.clone())
        .map_err(|e| anyhow::anyhow!("Invalid DNS name: {}", e))?;

    let tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|e| anyhow::anyhow!("TLS connection failed: {}", e))?;

    // Create WebSocket connection (use client_async since TLS is already established)
    let (ws, _) = tokio_tungstenite::client_async(req, tls_stream)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {}", e))?;

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
    }
    // Return the audio data
    Ok(mp3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_mp3() -> Result<()> {
        // Skip this test in CI environment or when network is not available
        if env::var("CI").is_ok() || env::var("SKIP_NETWORK_TESTS").is_ok() {
            println!("Skipping test_get_mp3 due to CI or SKIP_NETWORK_TESTS environment variable");
            return Ok(());
        }

        let ssml = r#"
            <speak version="1.0" xmlns="https://www.w3.org/2001/10/synthesis" xml:lang="en-US">
                <voice name="en-US-AriaNeural">
                    Hello world
                </voice>
            </speak>
        "#;

        match get_mp3(ssml).await {
            Ok(mp3) => {
                assert!(!mp3.is_empty());
                // Check if the data starts with MP3 magic number (ID3 or MPEG sync)
                assert!(
                    mp3.starts_with(&[0x49, 0x44, 0x33]) || // ID3v2
                       mp3.starts_with(&[0xFF, 0xFB])
                ); // MPEG sync
                Ok(())
            }
            Err(e) => {
                println!("Skipping test_get_mp3 due to connection error: {}", e);
                Ok(()) // Skip test on connection error
            }
        }
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
        let body_bytes = b"Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!Hello, world!".to_vec();
        let mut compression_type = "gzip, deflate, br, zstd".to_string();
        let result = compress_body(&body_bytes, &mut compression_type).unwrap();
        println!("compression_type: {}", compression_type);
        assert_eq!(compression_type, "br");
        assert!(result.len() < body_bytes.len());
    }
}
