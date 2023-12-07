use axum::{http::StatusCode, response::Html};
use lazy_static::lazy_static;
use prometheus::IntCounter;

lazy_static! {
    /// A prometheus metric counting the number of get requests to unknown URLs. It is accessible to
    /// clients via the `metrics` route.
    static ref NUM_404_REQUESTS: IntCounter =
        register_int_counter!("throttle_num_404", "Number of Get requests to unknown resource.")
            .expect("Error registering num_404 prometheus metric");
}

// Remark: We could use a statically hosted file, but then we would have to worry about deployment
// of static assets. It seems easier to just compile the static assets into the executable.
//
// use actix_files::NamedFile;
// pub fn not_found() -> std::io::Result<NamedFile> {
//     // Increment prometheous metric
//     NUM_404_REQUESTS.inc();
//     // Respond with static 404.html page
//     Ok(NamedFile::open("static/404.html")?)
// }
const NOT_FOUND_PAGE: &str = include_str!("404.html");

/// 404 handler
pub async fn not_found() -> (StatusCode, Html<&'static str>) {
    // Increment prometheous metric
    NUM_404_REQUESTS.inc();
    // Respond with static 404.html page
    (StatusCode::NOT_FOUND, Html(NOT_FOUND_PAGE))
}

/// Use this to initialize metrics eagerly, i.e. before the handler is called for the first time.
pub fn initialize_metrics() {
    lazy_static::initialize(&NUM_404_REQUESTS);
}
