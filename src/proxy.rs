use std::io::Read;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Request, Response, Uri};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use lazy_static::lazy_static;
use log::{debug, info};

use crate::{utils, AppContext};

async fn modify_response(
    context: Arc<AppContext>,
    // forward_uri: &str,
    resp: Response<Full<Bytes>>,
) -> Result<Response<Full<Bytes>>> {
    let mut res = Response::builder().status(resp.status());
    info!("modify response");
    debug!("status: {}", resp.status());
    let new_headers = res.headers_mut().context("failed to get headers")?;
    let mut need_compress = true;
    for (key, value) in resp.headers().iter() {
        let mut text = value.to_str()?.to_string();
        if key == hyper::header::SET_COOKIE {
            text = text.replace(".booklink.me", &context.host);
            debug!("set cookie: {}", text);
        }
        if key == hyper::header::LOCATION {
            need_compress = false;
            text = text.replace("http://www.wcxsw.org/", "https://m.wcxsw.org/");
            text = text.replace("http://www.wucuoxs.com", "https://m.wucuoxs.com");
            if text.starts_with("http") && !text.contains(&context.host) {
                if !context.port.is_empty() {
                    text = format!(
                        "{}://{}:{}/?dest={}",
                        &context.scheme, &context.host, &context.port, text
                    );
                } else {
                    text = format!("{}://{}/?dest={}", context.scheme, context.host, text);
                }
                new_headers.remove(hyper::header::LOCATION);
                debug!("redirect to: {}", text);
            }
        }
        new_headers.append(key, text.as_str().parse()?);
    }
    let mut body_bytes: Vec<u8> = resp.collect().await?.to_bytes().to_vec();
    if let Some(v) = new_headers.get(hyper::header::CONTENT_TYPE) {
        if v.to_str()?.contains("text") {
            let mut body_string = String::from_utf8(body_bytes)?;
            body_string = body_string.replace("google-analytics.com", "0.0.0.0");
            body_string = body_string.replace("adsbygoogle", "xxxxxxx");
            body_string = body_string.replace("<li class=\"hla\">", "<li class=\"\">");
            if body_string.contains("slist sec") {
                body_string = body_string.replace(
                    "<body>",
                    "<body><style>ul.list.sec {display: none;}</style>",
                );
            }
            // body_string =
            //     body_string.replace("www.google.com/search?ie=utf-8&", "duckduckgo.com/?ia=qa&");
            let script = r#"
                <script type="text/javascript">
                function chg(e) {
                    if (e === "dark") {
                        document.body.style.color = "white";
                        document.body.style.backgroundColor = "black";
                        var all = document.querySelectorAll('a');
                        var top = document.querySelectorAll('a.top');
                        [].slice.call(all).forEach(function(elem) {
                            elem.style.color = '#338dff';
                        });
                        [].slice.call(top).forEach(function(elem) {
                            elem.style.color = '#f00';
                        });
                        var elements = document.getElementsByClassName('grey');
                        [].slice.call(elements).forEach(function(elem) {
                            elem.style.color = '#a9a196';
                        });
                    } else {
                        document.body.style.color = "black";
                        document.body.style.backgroundColor = "white";
                        var all = document.querySelectorAll('a');
                        var top = document.querySelectorAll('a.top');
                        [].slice.call(all).forEach(function(elem) {
                            elem.style.color = '#03f';
                        });
                        [].slice.call(top).forEach(function(elem) {
                            elem.style.color = '#f00';
                        });
                        var elements = document.getElementsByClassName('grey');
                        [].slice.call(elements).forEach(function(elem) {
                            elem.style.color = '#646464';
                        });
                    }
                }
                if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
                    chg("dark");
                } else {
                    chg("light");
                }
                window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', event => {
                    const newColorScheme = event.matches ? "dark" : "light";
                    chg(newColorScheme);
                });
                </script>
                </body>
                "#;
            // body_string = body_string.replace("color=\"#646464\"", "color=\"#a9a196\"");
            body_string =
                body_string.replace("><font color=\"#646464\">", " class=\"grey\"><font>");
            body_string = body_string.replace("</body>", script);
            body_bytes = body_string.as_bytes().to_vec();
        }
    }
    if need_compress {
        body_bytes = utils::compress_body(&body_bytes)?;

        new_headers.remove(hyper::header::CONTENT_ENCODING);
        new_headers.remove(hyper::header::CONTENT_LENGTH);
        new_headers.insert(
            hyper::header::CONTENT_ENCODING,
            HeaderValue::from_static("br"),
        );
        debug!("compress body");
        new_headers.insert(
            hyper::header::CONTENT_LENGTH,
            body_bytes.len().to_string().parse()?,
        );
        debug!("set content length: {}", body_bytes.len());
    }
    Ok(res.body(Full::from(body_bytes))?)
}

fn is_hop_header(name: &str) -> bool {
    use unicase::Ascii;

    // A list of the headers, using `unicase` to help us compare without
    // worrying about the case, and `lazy_static!` to prevent reallocation
    // of the vector.
    lazy_static! {
        static ref HOP_HEADERS: Vec<Ascii<&'static str>> = vec![
            Ascii::new("Connection"),
            Ascii::new("Keep-Alive"),
            Ascii::new("Proxy-Authenticate"),
            Ascii::new("Proxy-Authorization"),
            Ascii::new("Te"),
            Ascii::new("Trailers"),
            Ascii::new("Transfer-Encoding"),
            Ascii::new("Upgrade"),
        ];
    }

    HOP_HEADERS.iter().any(|h| h == &name)
}

/// Returns a clone of the headers without the [hop-by-hop headers].
///
/// [hop-by-hop headers]: http://www.w3.org/Protocols/rfc2616/rfc2616-sec13.html
fn remove_hop_headers(headers: &mut HeaderMap<HeaderValue>) {
    for (k, _v) in headers.clone().iter() {
        if is_hop_header(k.as_str()) {
            headers.remove(k);
        }
    }
}

async fn create_proxied_response(
    mut response: Response<Incoming>,
) -> Result<Response<Full<Bytes>>> {
    // println!("original response: {:#?}", response);
    info!("create_proxied_response");
    remove_hop_headers(response.headers_mut());
    let (mut parts, body) = response.into_parts();
    let body_bytes = body.collect().await?.to_bytes().to_vec();
    let headers = parts.headers.clone();
    // let decoder = match headers.get(hyper::header::CONTENT_ENCODING) {
    //     Some(value) => {
    //         parts.headers.remove(hyper::header::CONTENT_ENCODING);
    //         value.to_str()?
    //     }
    //     None => "",
    // };
    // let decoder = if let Some(value) = headers.get(hyper::header::CONTENT_ENCODING) {
    //     parts.headers.remove(hyper::header::CONTENT_ENCODING);
    //     value.to_str()?
    // } else {
    //     ""
    // };
    // Determine the content encoding and decode the response body if necessary
    let decoder = headers
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|value| {
            parts.headers.remove(hyper::header::CONTENT_ENCODING);
            value.to_str().ok()
        })
        .unwrap_or_default();
    debug!("decoder: {}", &decoder);
    let mut decoded = match decoder {
        "gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(&body_bytes[..]);
            let mut buf = Vec::new();
            let _ = decoder.read_to_end(&mut buf);
            buf
        }
        "deflate" => {
            let mut decoder = flate2::read::DeflateDecoder::new(&body_bytes[..]);
            let mut buf = Vec::new();
            let _ = decoder.read_to_end(&mut buf);
            buf
        }
        "br" => {
            let mut decoder = brotli::Decompressor::new(&body_bytes[..], body_bytes.len());
            let mut buf = Vec::new();
            let _ = decoder.read_to_end(&mut buf);
            buf
        }
        _ => body_bytes.to_vec(),
    };

    // Convert the response body to UTF-8 encoding if it is an HTML document and not already UTF-8
    let content_type = headers
        .get(hyper::header::CONTENT_TYPE)
        .context("get content type error")?;
    let v = content_type.to_str()?;
    if v.contains("text/html") {
        let mut encoding = "GB18030";
        if v.contains("charset=") {
            let x = v.split("charset=").collect::<Vec<&str>>();
            let y = x[1];
            parts.headers.insert(
                hyper::header::CONTENT_TYPE,
                format!("{}charset=utf-8", x[0].to_owned()).parse()?,
            );
            encoding = y.trim();
        }
        let e = &encoding.to_lowercase();
        debug!("encoding: {}", &e);
        decoded = utils::to_utf8(&decoded, e)?.as_bytes().to_vec();
    }
    Ok(Response::from_parts(parts, decoded.into()))
}

fn forward_uri(forward_url: &str, req: &Request<Incoming>) -> Result<Uri> {
    if !forward_url.is_empty() {
        // let new_uri = match req.uri().query() {
        //     Some(query) => format!("{}{}?{}", forward_url, req.uri().path(), query),
        //     None => format!("{}{}", forward_url, req.uri().path()),
        // };
        let new_uri = if let Some(query) = req.uri().query() {
            format!("{}{}?{}", forward_url, req.uri().path(), query)
        } else {
            format!("{}{}", forward_url, req.uri().path())
        };
        Ok(Uri::from_str(new_uri.as_str())?)
    } else {
        Ok(req.uri().clone())
    }
}

fn create_proxied_request(
    context: Arc<AppContext>,
    forward_url: &str,
    mut request: Request<Incoming>,
) -> Result<Request<Incoming>> {
    remove_hop_headers(request.headers_mut());
    *request.uri_mut() = forward_uri(forward_url, &request)?;

    let host_val = request.uri().host().unwrap().to_string();
    request.headers_mut().remove(hyper::header::ACCEPT_ENCODING);
    request.headers_mut().insert(
        hyper::header::ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br"),
    );
    request
        .headers_mut()
        .insert(hyper::header::HOST, host_val.parse()?);

    request
        .headers_mut()
        .insert(hyper::header::USER_AGENT, context.ua.parse()?);
    Ok(request)
}

pub async fn call(
    context: Arc<AppContext>,
    forward_uri: &str,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>> {
    let proxied_request = create_proxied_request(context.clone(), forward_uri, request)?;
    let https = hyper_tls::HttpsConnector::new();
    // let client: Client<_, Body> = Client::builder().build(https);
    let client = Client::builder(TokioExecutor::new()).build(https);
    let response = client.request(proxied_request).await?;
    let proxied_response = create_proxied_response(response).await?;
    modify_response(context.clone(), proxied_response).await
}
