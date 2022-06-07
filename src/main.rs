use std::{collections::HashMap, convert::Infallible, net::SocketAddr};
use std::env;
use std::io::BufReader;
use std::sync::Arc;

use anyhow::{Context, Result};
use hyper::{Body, Request, Response, Server, service::{make_service_fn, service_fn}};
use hyper::server::conn::AddrStream;
use readability::extractor::{get_dom, Product};
use readability::markup5ever_arcdom::Node;
use readability::markup5ever_arcdom::NodeData::Element;
use regex::Regex;
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
                let body = fetch_novel(&dest)?;
                let mut p0 = get_content(&body[..], &Url::parse(&dest)?, &re)?;
                let mut next = p0.content;
                while !next.is_empty() {
                    let next_url = match next.contains("http") {
                        true => Url::parse(&next)?,
                        false => base.join(&next)?,
                    };
                    let resp_orig = fetch_novel(next_url.as_str())?;
                    let p1 = get_content(&resp_orig[..], &next_url, &re)?;
                    p0.text += &p1.text;
                    next = p1.content;
                }
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
                html.push_str(&p0.text);
                html.push_str(r#"</body></html>"#);

                let new_body = utils::compress_body(&html.into_bytes())?;
                let new_resp = Response::builder()
                    .status(hyper::http::StatusCode::OK)
                    .header(hyper::header::CONTENT_ENCODING, "br")
                    .header(hyper::header::CONTENT_LENGTH, new_body.len().to_string())
                    .body(Body::from(new_body))?;
                // let new_resp = Response::from_parts(parts, Body::from(new_body));
                Ok(new_resp)
            };
        }
    } else {
        let resp = proxy::call(context.clone(), &context.booksite, req).await;
        return resp;
    }
    Ok(Response::new(Body::from("Hello World")))
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

fn fetch_novel(url: &str) -> Result<Vec<u8>> {
    let html = sysreq::get(url)?;
    let mut len = html.len();
    if len > 1024 {
        len = 1024;
    }
    let tmpu8 = &html[0..len];
    let tmp = String::from_utf8_lossy(tmpu8).to_string().to_lowercase();
    let mut charset = "gb18030";
    if tmp.contains("charset=") && (tmp.contains("utf-8") || tmp.contains("utf8")) {
        charset = "utf-8";
    }

    utils::to_utf8(&html, charset)
}
