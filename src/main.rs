use std::env;
use std::io::BufReader;

use std::sync::Arc;
use std::{collections::HashMap, convert::Infallible, net::SocketAddr};

use anyhow::{Context, Result};

use hyper::server::conn::AddrStream;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use readability::extractor::{get_dom, Product};
use readability::markup5ever_arcdom::Node;
use readability::markup5ever_arcdom::NodeData::Element;
use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

mod proxy;
mod utils;

pub struct AppContext {
    booksite: String,
    fontsize: String,
    ua: String,
    host: String,
    port: String,
    scheme: String,
}

async fn handle(context: Arc<AppContext>, req: Request<Body>) -> Result<Response<Body>> {
    let params: HashMap<String, String> = req
        .uri()
        .query()
        .map(|v| {
            url::form_urlencoded::parse(v.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_else(HashMap::new);
    if params.contains_key("dest") {
        let dest = params["dest"].clone();
        if !dest.is_empty() {
            return if dest.contains("fkzww.net") {
                Response::builder()
                    .status(hyper::http::StatusCode::FOUND)
                    .header(hyper::header::LOCATION, dest)
                    .body(Body::empty())
                    .context("redirect")
            } else {
                let p0 = get_all_txt(dest).await?;
                //toWrite := `<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1.0" /><title>` +title + `</title></head><body><h3>` + title + `</h3><style> p{text-indent:2em; font-size:` + strconv.Itoa(FONTSIZE) +";}</style>\n" + content + `</body></html>`
                let mut html = String::from(
                    r#"<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1.0" /><title>"#,
                );
                html.push_str(&p0.title);
                html.push_str(r#"</title></head><body><h3>"#);
                html.push_str(&p0.title);
                html.push_str(r#"</h3><style> p{text-indent:2em; font-size:"#);
                html.push_str(context.fontsize.to_string().as_str());
                html.push_str(r#"px;}</style>"#);

                html.push_str(
                    r#"<div id="div1" style="text-align:center"><audio id="au"><source src = "data:audio/mpeg;base64,SUQzBAAAAAABEVRYWFgAAAAtAAADY29tbWVudABCaWdTb3VuZEJhbmsuY29tIC8gTGFTb25vdGhlcXVlLm9yZwBURU5DAAAAHQAAA1N3aXRjaCBQbHVzIMKpIE5DSCBTb2Z0d2FyZQBUSVQyAAAABgAAAzIyMzUAVFNTRQAAAA8AAANMYXZmNTcuODMuMTAwAAAAAAAAAAAAAAD/80DEAAAAA0gAAAAATEFNRTMuMTAwVVVVVVVVVVVVVUxBTUUzLjEwMFVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVf/zQsRbAAADSAAAAABVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVf/zQMSkAAADSAAAAABVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV"></audio><button id="listen" type="button" onclick="listen()">Listen</button></div>"#
                );
                html.push_str(&p0.text);
                let script = r#"
                <script type="text/javascript">
                function chg(e) {
                    if (e === "dark") {
                        document.body.style.color = "white";
                        document.body.style.backgroundColor = "black";
                    } else {
                        document.body.style.color = "black";
                        document.body.style.backgroundColor = "white";
                    }
                }
                if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
                    chg("dark");
                } else {
                    chg("bright");
                }
                window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => {
                    const newColorScheme = event.matches ? "dark" : "light";
                    chg(newColorScheme);
                });
                function listen() {
                    let full = window.location.href;
                    let dest = full.replace("dest=", "listen=");
                    let btn = document.getElementById("listen");
                    let div = document.getElementById("div1");
                    let au = document.getElementById("au");
                    au.autoplay = true;
                    try {
                        au.src=dest;
                        au.addEventListener("canplaythrough", (event) => {
                            /* the audio is now playable; play it if permissions allow */
                            au.play();
                          });
                        au.controls = true;
                        div.insertBefore(au, btn);
                        btn.style.display = "none";
                    } catch (e) {
                      alert(e.stack);
                }
            } 
            </script>
                </body></html>
                "#;
                html.push_str(script);

                let new_body = utils::compress_body(&html.into_bytes())?;
                let new_resp = Response::builder()
                    .status(hyper::http::StatusCode::OK)
                    .header(hyper::header::CONTENT_TYPE, "text/html")
                    .header(hyper::header::CONTENT_ENCODING, "br")
                    .header(hyper::header::CONTENT_LENGTH, new_body.len().to_string())
                    .body(Body::from(new_body))?;
                Ok(new_resp)
            };
        }
    } else if params.contains_key("listen") {
        let listen = params["listen"].clone();
        let mut all = get_all_txt(listen).await?.text;
        all = all.replace("<p>", "");
        all = all.replace("</p>", "");
        // let lines = all.lines().collect::<Vec<&str>>();
        // let total_str = lines.filter(|&x| !x.is_empty()).collect::<Vec<&str>>();
        let size = 2500;
        // let n = lines.len() / size + 1;
        let all_chars = all.as_str().graphemes(true).collect::<Vec<&str>>();
        // dbg!(all_chars.len());
        // let chunks = all.len() / size + 1;
        let start = r#"<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xmlns:mstts="https://www.w3.org/2001/mstts" xml:lang="zh-CN"> <voice name="zh-CN-XiaoxiaoNeural"> <prosody rate="+50.00%">"#;
        let end = r#"</prosody> </voice> </speak>"#;
        let mut mp3: Vec<u8> = Vec::new();
        for chunk in all_chars.chunks(size) {
            let mut ssml = String::from(start);
            ssml.push_str(&chunk.join(""));
            ssml.push_str(end);
            let t = utils::get_token().await?;
            mp3.extend_from_slice(&utils::get_mp3(&t, &ssml).await?);
        }
        let mut resp = Response::new(Body::from(mp3));
        resp.headers_mut()
            .append(hyper::header::CONTENT_TYPE, "audio/mpeg".parse()?);
        return Ok(resp);
    } else {
        let resp = proxy::call(context.clone(), &context.booksite, req).await;
        return resp;
    }
    Ok(Response::new(Body::from("Hello World")))
}

async fn get_all_txt(dest: String) -> Result<Product, anyhow::Error> {
    let base = Url::parse(&dest)?;
    let mut re: Regex = Regex::new(r"xxx")?;
    if dest.contains('/') && dest.contains('.') {
        let first = dest.rfind('/').unwrap() + 1;
        let last = dest.rfind('.').unwrap();
        if first < last {
            let base = &dest[first..last];
            re = Regex::new(format!("{}_\\d+", base).as_str())?;
        }
    }
    let body = fetch_novel(&dest).await?;
    let mut p0 = get_content(&body[..], &Url::parse(&dest)?, &re)?;
    let mut next = p0.content.clone();
    while !next.is_empty() {
        let next_url = match next.contains("http") {
            true => Url::parse(&next)?,
            false => base.join(&next)?,
        };
        let resp_orig = fetch_novel(next_url.as_str()).await?;
        let p1 = get_content(&resp_orig[..], &next_url, &re)?;
        p0.text += &p1.text;
        next = p1.content;
    }
    Ok(p0)
}

#[tokio::main]
async fn main() {
    let debug = env::var("DEBUG").is_ok();
    let localport = env::var("LOCAL_PORT").unwrap_or_else(|_| "9005".to_string());
    let listenlocal = env::var("LISTEN_LOCAL").is_ok();
    let listenaddr = match listenlocal {
        true => [127, 0, 0, 1],
        false => [0, 0, 0, 0],
    };
    let context = AppContext {
        booksite: "https://m.booklink.me".to_string(),
        fontsize: env::var("FONTSIZE").unwrap_or_else(|_| "17".to_string()),
        ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.102 Safari/537.36".to_string(),
        host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        port: match debug {
            true => localport.clone(),
            false => env::var("PORT").unwrap_or_else(|_| "".to_string()),
        },
        scheme: env::var("SCHEME").unwrap_or_else(|_| "http".to_string()),
    };
    let c = Arc::new(context);
    // A `MakeService` that produces a `Service` to handle each connection.
    let make_service = make_service_fn(move |conn: &AddrStream| {
        // We have to clone the context to share it with each invocation of
        // `make_service`. If your data doesn't implement `Clone` consider using
        // an `std::sync::Arc`.
        let context = c.clone();

        // You can grab the address of the incoming connection like so.
        let _addr = conn.remote_addr();

        // Create a `Service` for responding to the request.
        let service = service_fn(move |req| handle(context.clone(), req));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    // Run the server like above...
    let addr = SocketAddr::from((listenaddr, localport.parse().unwrap()));

    let server = Server::bind(&addr).serve(make_service);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

fn get_next_link(node: Arc<Node>, re: &Regex) -> String {
    let handle = node;
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
        let next = get_next_link(c, re);
        if !next.is_empty() {
            return next;
        }
    }
    "".to_string()
}

fn get_content(content: &[u8], url: &Url, re: &Regex) -> Result<Product> {
    let mut bf = BufReader::new(content);
    let dom = get_dom(&mut bf)?;

    let next = get_next_link(dom.document.clone(), re);
    let mut p = readability::extractor::extract(dom, url)?;
    p.content = next;
    Ok(p)
}

async fn fetch_novel(url: &str) -> Result<Vec<u8>> {
    let output = tokio::process::Command::new("curl")
        .arg("-gL")
        .arg("--compressed")
        .arg("-A 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.102 Safari/537.36'")
        .arg(url)
        .output()
        .await?;
    let html = output.stdout;
    let mut len = html.len();
    if len > 1024 {
        len = 1024;
    }
    let tmp = String::from_utf8_lossy(&html[0..len])
        .to_string()
        .to_lowercase();
    let mut charset = "gb18030";
    if tmp.contains("charset=") && (tmp.contains("utf-8") || tmp.contains("utf8")) {
        charset = "utf-8";
    }

    utils::to_utf8(&html, charset)
}
