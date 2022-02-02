use actix_web::{get, HttpResponse};

/// Favicon is statically linkend into the binary. This saves us the headache of deploying static
/// assets.
const FAVICON: &[u8] = include_bytes!("favicon.ico");

/// Browsers like to ask for an Icon. We do not want to clutter our logs with meaningless 404s so we
/// just provide one.
#[get("/favicon.ico")]
pub async fn favicon() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("image/x-icon")
        .body(FAVICON)
}
