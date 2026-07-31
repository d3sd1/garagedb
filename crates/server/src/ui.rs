//! UI embebida en el binario: sin CDN, sin red en runtime.

use axum::response::{Html, IntoResponse};

pub async fn index() -> impl IntoResponse {
    Html(include_str!("../ui/index.html"))
}

pub async fn app_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        include_str!("../ui/app.js"),
    )
}
