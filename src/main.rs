use anyhow::Result;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use log::{debug, info};
use readability::extractor::{get_dom, Product};
use readability::markup5ever_rcdom::Node;
use readability::markup5ever_rcdom::NodeData::Element;
use regex::Regex;
use std::collections::VecDeque;
use std::env;
use std::io::BufReader;

use std::rc::Rc;
use std::sync::Arc;
use std::{collections::HashMap, net::SocketAddr};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::net::TcpListener;
use tokio::task;
use url::Url;
mod proxy;
mod utils;

#[derive(Debug)]
pub struct AppContext {
    booksite: String,
    fontsize: String,
    ua: String,
    host: String,
    port: String,
    scheme: String,
}

async fn handle(
    context: Arc<AppContext>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>> {
    let params: HashMap<String, String> = req
        .uri()
        .query()
        .map(|v| {
            url::form_urlencoded::parse(v.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();
    if let Some(dest) = params.get("dest").cloned().filter(|d| !d.is_empty()) {
        info!("dest: {}", &dest);
        if dest.contains("fkzww.net") {
            info!("fkzww.net: redirect");
            let r = Response::builder()
                .status(hyper::http::StatusCode::FOUND)
                .header(hyper::header::LOCATION, dest)
                .body(Full::new(Bytes::new()))?;
            return Ok(r);
        } else {
            let p0 = get_all_txt(dest).await?;
            let html = format!(
                r#"<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1.0" /><title>{title}</title></head><body><h3>{title}</h3><style> p{{text-indent:2em; font-size:{fontsize}px;}}</style><div id="div1" style="text-align:center"><audio id="au"><source src = "data:audio/mpeg;base64,SUQzBAAAAAABEVRYWFgAAAAtAAADY29tbWVudABCaWdTb3VuZEJhbmsuY29tIC8gTGFTb25vdGhlcXVlLm9yZwBURU5DAAAAHQAAA1N3aXRjaCBQbHVzIMKpIE5DSCBTb2Z0d2FyZQBUSVQyAAAABgAAAzIyMzUAVFNTRQAAAA8AAANMYXZmNTcuODMuMTAwAAAAAAAAAAAAAAD/80DEAAAAA0gAAAAATEFNRTMuMTAwVVVVVVVVVVVVVUxBTUUzLjEwMFVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVf/zQsRbAAADSAAAAABVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVf/zQMSkAAADSAAAAABVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV"></audio><button id="listen" type="button" onclick="listen()">Listen</button></div>{content}
<script type="text/javascript">
function chg(e) {{
    if (e === "dark") {{
        document.body.style.color = "white";
        document.body.style.backgroundColor = "black";
    }} else {{
        document.body.style.color = "black";
        document.body.style.backgroundColor = "white";
    }}
}}
if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {{
    chg("dark");
}} else {{
    chg("bright");
}}
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => {{
    const newColorScheme = event.matches ? "dark" : "light";
    chg(newColorScheme);
}});
function listen() {{
    let full = window.location.href;
    let dest = full.replace("dest=", "listen=");
    let btn = document.getElementById("listen");
    let div = document.getElementById("div1");
    let au = document.getElementById("au");
    au.autoplay = true;
    try {{
        au.src=dest;
        au.addEventListener("canplaythrough", (event) => {{
            au.play();
        }});
        au.controls = true;
        div.insertBefore(au, btn);
        btn.style.display = "none";
    }} catch (e) {{
        alert(e.stack);
    }}
}}
</script>
</body></html>"#,
                title = p0.title,
                fontsize = context.fontsize,
                content = p0.text
            );
            debug!("html: {}", &html);
            let mut encoding = req
                .headers()
                .get(hyper::header::ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let new_body = utils::compress_body(&html.into_bytes(), &mut encoding)?;
            let new_resp = Response::builder()
                .status(hyper::http::StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "text/html")
                .header(hyper::header::CONTENT_ENCODING, &encoding)
                .header(hyper::header::CONTENT_LENGTH, new_body.len().to_string())
                .body(Full::from(new_body))?;
            return Ok(new_resp);
        }
    } else if let Some(listen) = params.get("listen").cloned() {
        let mut all = get_all_txt(listen).await?.text;
        all = all.replace("</p>", "");
        let lines = all.split("<p>").collect::<Vec<&str>>();
        let n = 10;
        let start = r#"<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xmlns:mstts="https://www.w3.org/2001/mstts" xml:lang="zh-CN"> <voice name="zh-CN-XiaoxiaoNeural"> <prosody rate="+50.00%">"#;
        let end = r#"</prosody> </voice> </speak>"#;
        let mut mp3 = Vec::new();
        let size = lines.len() / n;
        debug!("size={size}");
        let mut handles = Vec::new();
        for i in 0..n {
            let mut ssml = String::from(start);
            let s = if i == n - 1 {
                lines[i * size..].join("")
            } else {
                lines[i * size..(i + 1) * size].join("")
            };
            ssml.push_str(&s);
            ssml.push_str(end);
            debug!("ssml: {}", &ssml);
            let handle = task::spawn(async move { utils::get_mp3(&ssml).await });
            handles.push(handle);
        }
        for handle in handles {
            if let Ok(result) = handle.await {
                mp3.extend_from_slice(&result?);
            }
        }
        let mut resp = Response::new(Full::from(mp3));
        resp.headers_mut()
            .append(hyper::header::CONTENT_TYPE, "audio/mpeg".parse()?);
        return Ok(resp);
    }
    proxy::call(context.clone(), &context.booksite, req).await
}

async fn get_all_txt(dest: String) -> Result<Product> {
    let base = Url::parse(&dest)?;
    let re = if let (Some(first_idx), Some(last_idx)) = (dest.rfind('/'), dest.rfind('.')) {
        let first = first_idx + 1;
        if first < last_idx {
            let base_name = &dest[first..last_idx];
            Regex::new(&format!("{base_name}[_-]\\d+"))?
        } else {
            Regex::new(r"^$")? // Never matches
        }
    } else {
        Regex::new(r"^$")? // Never matches
    };
    let body = fetch_novel(&dest).await?;
    let mut p0 = get_content(&body[..], &Url::parse(&dest)?, &re)?;
    let mut next = p0.content.clone();
    while !next.is_empty() {
        debug!("next: {}", &next);
        let next_url = if next.contains("http") {
            Url::parse(&next)?
        } else {
            base.join(&next)?
        };
        let resp_orig = fetch_novel(next_url.as_str()).await?;
        let p1 = get_content(&resp_orig[..], &next_url, &re)?;
        p0.text += &p1.text;
        next = p1.content;
    }
    Ok(p0)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dev = env::var("DEV").is_ok();
    let log_level = if dev {
        "debug".to_string()
    } else {
        env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&log_level)).init();
    let localport = env::var("LOCAL_PORT").unwrap_or_else(|_| "9005".to_string());
    let listenlocal = env::var("LISTEN_LOCAL").is_ok();
    let listenaddr = match listenlocal {
        true => [127, 0, 0, 1],
        false => [0, 0, 0, 0],
    };
    let context = AppContext {
        booksite: "https://m.booklink.me".to_string(),
        fontsize: env::var("FONTSIZE").unwrap_or_else(|_| "17".to_string()),
        ua: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36".to_string(),
        host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: if dev {
            localport.clone()
        }else{
            env::var("PORT").unwrap_or_else(|_| "".to_string())
        },
        scheme: env::var("SCHEME").unwrap_or_else(|_| "http".to_string()),
    };
    info!("context: {:?}", &context);
    let c = Arc::new(context);

    // Run the server like above...
    let addr = SocketAddr::from((listenaddr, localport.parse().unwrap()));
    let listener = TcpListener::bind(addr).await?;
    info!("Listening on: {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let c = c.clone();
        let service = service_fn(move |req| handle(c.clone(), req));

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                println!("Failed to serve connection: {:?}", err);
            }
        });
    }
}

fn get_next_link(node: Rc<Node>, re: &Regex) -> String {
    let mut queue = VecDeque::new();
    queue.push_back(node);

    while let Some(handle) = queue.pop_front() {
        for child in handle.children.borrow().iter() {
            let c = child.clone();
            if let Element {
                ref name,
                ref attrs,
                ..
            } = c.data
            {
                if name.local.as_ref() == "a" {
                    for h in attrs.borrow().iter() {
                        if h.name.local.as_ref() == "href" {
                            let url = h.value.to_string().to_owned();
                            if url.len() >= 4 && url.contains('/') && url.contains('.') {
                                let first = url.rfind('/').unwrap() + 1;
                                let last = url.rfind('.').unwrap();
                                if first < last {
                                    let dest = &url[first..last];
                                    if re.is_match(dest) {
                                        return url;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            queue.push_back(c);
        }
    }
    "".to_string()
}

fn get_content(content: &[u8], url: &Url, re: &Regex) -> Result<Product> {
    let mut bf = BufReader::new(content);
    let dom = get_dom(&mut bf)?;

    let next = get_next_link(dom.document.clone(), re);
    debug!("next: {}", &next);
    let mut p = readability::extractor::extract(dom, url)?;
    p.content = next;
    Ok(p)
}

async fn fetch_novel(url: &str) -> Result<Vec<u8>> {
    use http_body_util::BodyExt;
    use hyper_util::{client::legacy::Client, rt::TokioExecutor};
    use std::io::Read;

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();
    let client = Client::builder(TokioExecutor::new()).build(https);

    let mut current_url = url.to_string();
    let max_redirects = 10;

    for _ in 0..max_redirects {
        let req = hyper::Request::builder()
            .method("GET")
            .uri(&current_url)
            .header(hyper::header::USER_AGENT, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")
            .header(hyper::header::ACCEPT_ENCODING, "gzip, deflate, br")
            .body(http_body_util::Empty::<Bytes>::new())?;

        let resp: hyper::Response<hyper::body::Incoming> = client.request(req).await?;
        let status = resp.status();

        if status.is_redirection() {
            if let Some(location) = resp.headers().get(hyper::header::LOCATION) {
                let location_str = location.to_str()?;
                current_url = if location_str.starts_with("http") {
                    location_str.to_string()
                } else {
                    let base = Url::parse(&current_url)?;
                    base.join(location_str)?.to_string()
                };
                debug!("Redirecting to: {}", current_url);
                continue;
            }
        }

        let encoding = resp
            .headers()
            .get(hyper::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body_bytes = resp.collect().await?.to_bytes().to_vec();

        let html = match encoding.as_str() {
            "gzip" => {
                let mut decoder = flate2::read::GzDecoder::new(&body_bytes[..]);
                let mut buf = Vec::new();
                decoder.read_to_end(&mut buf)?;
                buf
            }
            "deflate" => {
                let mut decoder = flate2::read::DeflateDecoder::new(&body_bytes[..]);
                let mut buf = Vec::new();
                decoder.read_to_end(&mut buf)?;
                buf
            }
            "br" => {
                let mut decoder = brotli::Decompressor::new(&body_bytes[..], body_bytes.len());
                let mut buf = Vec::new();
                decoder.read_to_end(&mut buf)?;
                buf
            }
            _ => body_bytes,
        };

        let r = if let Ok(r) = String::from_utf8(html.clone()) {
            r
        } else {
            utils::to_utf8(&html, "gb18030")?
        };

        return Ok(r.as_bytes().to_vec());
    }

    Err(anyhow::anyhow!("Too many redirects"))
}
