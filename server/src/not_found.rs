use axum::{http::StatusCode, response::Html};

// Remark: We could use a statically hosted file, but then we would have to worry about deployment
// of static assets. It seems easier to just compile the static assets into the executable.
const NOT_FOUND_PAGE: &str = include_str!("404.html");

/// 404 handler
pub async fn not_found() -> (StatusCode, Html<&'static str>) {
    // Respond with static 404.html page
    (StatusCode::NOT_FOUND, Html(NOT_FOUND_PAGE))
}
