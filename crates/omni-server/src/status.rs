// Embedded Interactive Web Dashboard SPA — compiled directly into the single Rust binary
// via `include_str!("dashboard.html")`. Zero external CDN assets, 100% offline-capable.

use axum::http::header;
use axum::response::Response;

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

pub async fn status_page() -> Response {
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(DASHBOARD_HTML))
        .unwrap()
}
