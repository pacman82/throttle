use axum::Router;
use memory_serve::{load_assets, MemoryServe};

pub fn frontend_router() -> Router {
    let assets = load_assets!("../ui/build");
    MemoryServe::new(assets).index_file(Some("/index.html")).into_router()
}