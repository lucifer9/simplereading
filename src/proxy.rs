use std::io;
use std::io::Read;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use encoding::DecoderTrap;
use encoding::label::encoding_from_whatwg_label;
use follow_redirects::ClientExt;
use hyper::{Body, Client, Request, Response, Uri};
use hyper::header::{HeaderMap, HeaderValue};
use lazy_static::lazy_static;

use crate::{AppContext, utils};

async fn modify_response(
    context: Arc<AppContext>,
    forward_uri: &str,
    resp: Response<Body>,
) -> Result<Response<Body>> {
    if !context.booksite.eq_ignore_ascii_case(forward_uri) {
        return Ok(resp);
    }
    let mut res = Response::builder().status(resp.status());
    let new_headers = res.headers_mut().context("failed to get headers")?;
    let mut need_compress = true;
    for (key, value) in resp.headers().iter() {
        let mut text = value.to_str()?.to_string();
        if key == hyper::header::SET_COOKIE {
            text = text.replace(".booklink.me", &context.host);
        }
        if key == hyper::header::LOCATION {
            need_compress = false;
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
            }
        }
        new_headers.append(key, text.as_str().parse()?);
    }
    let mut body_bytes = hyper::body::to_bytes(resp.into_body())
        .await
        .context("body to bytes error")?
        .to_vec();
    match new_headers.get(hyper::header::CONTENT_TYPE) {
        Some(v) => {
            if v.to_str()?.contains("text") {
                let mut body_string = String::from_utf8(body_bytes).unwrap();
                body_string = body_string.replace("adsbygoogle", "xxxxxxx");
                body_string = body_string.replace("<li class=\"hla\">", "<li class=\"\">");
                if body_string.contains("slist sec") {
                    body_string = body_string
                        .replace("<body>", "<style>ul.list.sec {display: none;}</style>");
                }
                body_string = body_string
                    .replace("www.google.com/search?ie=utf-8&", "duckduckgo.com/?ia=qa&");
                // return Ok(res.body(Body::from(body_string)).unwrap());
                body_bytes = body_string.as_bytes().to_vec();
            }
        }
        None => {}
    }
    if need_compress {
        body_bytes = utils::compress_body(new_headers, &body_bytes)?;
    }
    res.body(Body::from(body_bytes))
        .context("modify response error")
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

async fn create_proxied_response(mut response: Response<Body>) -> Result<Response<Body>> {
    // println!("original response: {:#?}", response);
    remove_hop_headers(response.headers_mut());
    let (mut parts, body) = response.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;
    let headers = parts.headers.clone();
    let decoder = match headers.get(hyper::header::CONTENT_ENCODING) {
        Some(value) => {
            parts.headers.remove(hyper::header::CONTENT_ENCODING);
            value.to_str()?
        }
        None => "",
    };
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
            let mut decoder = brotli2::read::BrotliDecoder::new(&body_bytes[..]);
            let mut buf = Vec::new();
            let _ = decoder.read_to_end(&mut buf);
            buf
        }
        _ => body_bytes.to_vec(),
    };

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
                (x[0].to_owned() + "charset=utf-8").parse()?,
            );
            encoding = y.trim();
        }
        let e = &encoding.to_lowercase();
        let e1 = encoding_from_whatwg_label(e).context("encoding error")?;
        let s = e1.decode(&decoded[..], DecoderTrap::Strict).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("decode error: {}", encoding),
            )
        })?;
        decoded = s.into_bytes();
    }
    Ok(Response::from_parts(parts, decoded.into()))
}

fn forward_uri(forward_url: &str, req: &Request<Body>) -> Result<Uri> {
    if !forward_url.is_empty() {
        let new_uri = match req.uri().query() {
            Some(query) => format!("{}{}?{}", forward_url, req.uri().path(), query),
            None => format!("{}{}", forward_url, req.uri().path()),
        };

        Ok(Uri::from_str(new_uri.as_str())?)
    } else {
        Ok(req.uri().clone())
    }
}

fn create_proxied_request(
    context: Arc<AppContext>,
    forward_url: &str,
    mut request: Request<Body>,
) -> Result<Request<Body>> {
    remove_hop_headers(request.headers_mut());
    *request.uri_mut() = forward_uri(forward_url, &request)?;

    let host_val = request.uri().host().unwrap().to_string();
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
    request: Request<Body>,
) -> Result<Response<Body>> {
    let proxied_request = create_proxied_request(context.clone(), forward_uri, request)?;
    let https = hyper_tls::HttpsConnector::new();
    let client: Client<_, Body> = Client::builder().build(https);
    let response = match context.booksite.eq_ignore_ascii_case(forward_uri) {
        true => client.request(proxied_request).await?,
        false => {
            client
                .follow_redirects_max(10)
                .request(proxied_request)
                .await?
        }
    };
    let proxied_response = create_proxied_response(response).await?;
    let p = modify_response(context.clone(), forward_uri, proxied_response).await;
    p
}
