// Embedded React + Vite + Tailwind v4 Web Dashboard SPA & 146 official provider SVG logos.
// Compiled directly into the single native Rust binary via `rust-embed` (zero Node.js runtime,
// zero external CDN requests, ~0 MB extra RAM on low-memory ARM64 servers).

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::Response;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

fn serve_embedded(path: &str, immutable: bool) -> Response {
    match WebAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let cache = if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .body(Body::from(file.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("asset not found"))
            .unwrap(),
    }
}

pub async fn status_page() -> Response {
    serve_embedded("index.html", false)
}

pub async fn favicon_asset() -> Response {
    serve_embedded("favicon.svg", true)
}

pub async fn static_asset(Path(path): Path<String>) -> Response {
    let clean = path.trim_start_matches('/');
    serve_embedded(&format!("assets/{clean}"), true)
}

pub async fn provider_icon_asset(Path(path): Path<String>) -> Response {
    let clean = path.trim_start_matches('/');
    serve_embedded(&format!("providers/{clean}"), true)
}
