//! The app's files, from the copy built into the binary or from a directory given with `--static`, with one set of
//! headers for both: how long a browser may keep each file, and which encoding it was sent in.

use std::path::Path;

use acyclic_cli::error::{CliError, exit};
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, ETAG, IF_NONE_MATCH, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

/// What every page is allowed to load and do: only the server's own files and API, nothing framed, no other base,
/// and forms only to itself. Inline styles stay allowed, since the graph's renderers set them; inline scripts don't.
pub const POLICY: &str = "default-src 'self'; script-src 'self'; worker-src 'self'; connect-src 'self'; \
                          img-src 'self' data:; style-src 'self' 'unsafe-inline'; manifest-src 'self'; \
                          frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// The bundle's files, whose names carry a hash of their content, so a browser may keep them for good.
const HASHED: &str = "/assets/";

/// Checks a `--static` directory before serving from it: it must hold the app's `index.html`, so a directory left
/// empty, such as one an image made for it, fails at start rather than answering every page with 404.
///
/// # Errors
///
/// A usage error naming the directory.
pub fn check_dir(dir: &Path) -> Result<(), CliError> {
    if dir.join("index.html").is_file() {
        Ok(())
    } else {
        Err(CliError::new(
            "usage",
            exit::USAGE,
            format!(
                "--static {} has no index.html: build the app into it, or leave --static out",
                dir.display()
            ),
        ))
    }
}

/// Serves a file of the app built into the binary: GET and HEAD only, `/` as `/index.html`, in the best encoding the
/// browser accepts that the build wrote.
pub fn embedded(request: &Request) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = match request.uri().path() {
        "/" => "/index.html",
        path => path,
    };
    let Some(asset) = acyclic_app_assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let accepted = accepted(request.headers());
    let (body, encoding) = match (asset.br, asset.gz) {
        (Some(br), _) if accepted.br => (br, Some("br")),
        (_, Some(gz)) if accepted.gzip => (gz, Some("gzip")),
        _ => (asset.body, None),
    };
    // Each encoding is its own representation, with its own tag.
    let tag = format!("\"{}-{}\"", asset.tag, encoding.unwrap_or("identity"));
    let unchanged = request.headers().get_all(IF_NONE_MATCH).iter().any(|sent| {
        sent.to_str().unwrap_or_default().split(',').any(|one| {
            let one = one.trim();
            one == "*" || one.strip_prefix("W/").unwrap_or(one) == tag
        })
    });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else if request.method() == Method::HEAD {
        Response::new(Body::empty())
    } else {
        Response::new(Body::from(body))
    };
    let headers = response.headers_mut();
    // A tag is hex and an encoding's name, always a valid value.
    if let Ok(tag) = HeaderValue::from_str(&tag) {
        headers.insert(ETAG, tag);
    }
    if !unchanged {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type(path)));
        if let Some(encoding) = encoding {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_static(encoding));
        }
    }
    response
}

/// Sets the headers every file of the app carries, wherever it came from: how long it may be kept, and that its
/// encoding depends on `Accept-Encoding`, on what it answers and on a 304 alike. Refusals and errors are left alone.
pub fn cache(path: &str, response: &mut Response) {
    if !response.status().is_success() && response.status() != StatusCode::NOT_MODIFIED {
        return;
    }
    let headers = response.headers_mut();
    let policy =
        if path.starts_with(HASHED) { "public, max-age=31536000, immutable" } else { "no-cache" };
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(policy));
    headers.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
}

/// A static directory's server: precompressed copies first, as the embedded app has them.
pub(crate) fn dir(dir: &Path) -> tower_http::services::ServeDir {
    tower_http::services::ServeDir::new(dir).precompressed_br().precompressed_gzip()
}

/// The encodings a request accepts, by `Accept-Encoding`, where `q=0` refuses one.
#[derive(Debug, Default, PartialEq, Eq)]
struct Accepted {
    br: bool,
    gzip: bool,
}

fn accepted(headers: &HeaderMap) -> Accepted {
    let mut accepted = Accepted::default();
    for value in headers.get_all(ACCEPT_ENCODING) {
        for entry in value.to_str().unwrap_or_default().split(',') {
            let mut parts = entry.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts.any(|param| {
                param.strip_prefix("q=").is_some_and(|q| q.parse::<f32>().is_ok_and(|q| q <= 0.0))
            });
            match name.to_ascii_lowercase().as_str() {
                "br" => accepted.br = !refused,
                "gzip" => accepted.gzip = !refused,
                _ => {},
            }
        }
    }
    accepted
}

/// The type of an app file, by its extension.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map_or("", |(_, ext)| ext) {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, value.parse().unwrap());
        headers
    }

    #[test]
    fn encodings_follow_accept_encoding_and_q_zero_refuses() {
        assert_eq!(accepted(&HeaderMap::new()), Accepted::default());
        assert_eq!(
            accepted(&headers("gzip, deflate, br, zstd")),
            Accepted { br: true, gzip: true }
        );
        assert_eq!(accepted(&headers("br;q=0, GZIP;q=0.5")), Accepted { br: false, gzip: true });
        assert_eq!(accepted(&headers("br;q=0.0")), Accepted::default());
    }

    #[test]
    fn hashed_files_are_kept_for_good_and_the_rest_revalidated() {
        let kept = |path: &str, status: StatusCode| {
            let mut response = status.into_response();
            cache(path, &mut response);
            let headers = response.headers();
            (headers.get(CACHE_CONTROL).cloned(), headers.get(VARY).cloned())
        };
        let forever = HeaderValue::from_static("public, max-age=31536000, immutable");
        let fresh = HeaderValue::from_static("no-cache");
        let vary = Some(HeaderValue::from_static("Accept-Encoding"));
        assert_eq!(kept("/assets/index-abc.js", StatusCode::OK), (Some(forever), vary.clone()));
        for path in ["/", "/index.html", "/sw.js", "/manifest.webmanifest"] {
            assert_eq!(kept(path, StatusCode::OK), (Some(fresh.clone()), vary.clone()), "{path}");
        }
        assert_eq!(kept("/sw.js", StatusCode::NOT_MODIFIED), (Some(fresh), vary));
        assert_eq!(kept("/assets/missing.js", StatusCode::NOT_FOUND), (None, None));
    }

    #[test]
    fn types_come_from_the_extension() {
        assert_eq!(content_type("/index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("/assets/index-abc.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("/manifest.webmanifest"), "application/manifest+json");
        assert_eq!(content_type("/icon.svg"), "image/svg+xml");
        assert_eq!(content_type("/README"), "application/octet-stream");
    }

    fn get(method: Method, path: &str, encodings: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        get_if(method, path, encodings, None)
    }

    fn get_if(
        method: Method,
        path: &str,
        encodings: &str,
        tag: Option<&HeaderValue>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request =
            Request::builder().method(method).uri(path).header(ACCEPT_ENCODING, encodings);
        if let Some(tag) = tag {
            request = request.header(IF_NONE_MATCH, tag);
        }
        let request = request.body(Body::empty()).unwrap();
        let response = embedded(&request);
        let (parts, body) = response.into_parts();
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let body = runtime.block_on(axum::body::to_bytes(body, usize::MAX)).unwrap();
        (parts.status, parts.headers, body.to_vec())
    }

    #[test]
    fn the_built_in_app_answers_get_and_head_in_an_encoding_the_browser_takes() {
        let (status, headers, body) = get(Method::GET, "/", "");
        assert_eq!(
            (status, &headers[CONTENT_TYPE]),
            (StatusCode::OK, &HeaderValue::from_static("text/html; charset=utf-8"))
        );
        assert_eq!(body, acyclic_app_assets::get("/index.html").unwrap().body);
        let (status, _, body) = get(Method::HEAD, "/index.html", "");
        assert_eq!((status, body.len()), (StatusCode::OK, 0));
        assert_eq!(get(Method::POST, "/index.html", "").0, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(get(Method::GET, "/missing", "").0, StatusCode::NOT_FOUND);
        // Without a build, the stub page has no compressed copies, and every file is sent as it is.
        for asset in acyclic_app_assets::FILES {
            let sent = |encodings| {
                let (status, headers, body) = get(Method::GET, asset.path, encodings);
                assert_eq!(status, StatusCode::OK, "{}", asset.path);
                (headers.get(CONTENT_ENCODING).cloned(), body)
            };
            let br = asset.br.map(|br| (Some(HeaderValue::from_static("br")), br.to_vec()));
            let gz = asset.gz.map(|gz| (Some(HeaderValue::from_static("gzip")), gz.to_vec()));
            let plain = (None, asset.body.to_vec());
            assert_eq!(
                sent("gzip, br"),
                br.clone().or_else(|| gz.clone()).unwrap_or_else(|| plain.clone())
            );
            assert_eq!(sent("gzip"), gz.unwrap_or_else(|| plain.clone()));
            assert_eq!(sent("identity"), plain);
        }
    }

    #[test]
    fn a_file_the_browser_has_is_not_modified_in_the_same_encoding() {
        let (_, headers, _) = get(Method::GET, "/index.html", "gzip, br");
        let tag = headers[ETAG].clone();
        let (status, headers, body) = get_if(Method::GET, "/index.html", "gzip, br", Some(&tag));
        assert_eq!((status, &headers[ETAG], body.len()), (StatusCode::NOT_MODIFIED, &tag, 0));
        assert_eq!(
            get_if(Method::GET, "/", "br, gzip", Some(&HeaderValue::from_static("*"))).0,
            StatusCode::NOT_MODIFIED
        );
        // Another encoding is another representation; without a build there is only one.
        let other = get_if(Method::GET, "/index.html", "identity", Some(&tag)).0;
        let compressed = acyclic_app_assets::get("/index.html").unwrap().gz.is_some();
        assert_eq!(other == StatusCode::OK, compressed);
    }

    #[test]
    fn a_static_dir_must_hold_the_app() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_dir(dir.path()).unwrap_err().code, "usage");
        std::fs::write(dir.path().join("index.html"), "the app").unwrap();
        assert!(check_dir(dir.path()).is_ok());
    }
}
