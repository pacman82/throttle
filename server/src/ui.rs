use memory_serve::{load_assets, MemoryServe};
use tokio::{spawn, task::JoinHandle};
use std::io;

pub struct Frontend {
    join_handle: JoinHandle<io::Result<()>>,
}

impl Frontend {
    pub async fn new(endpoint: &str) -> Result<Self, io::Error> {
        let assets = load_assets!("../ui/build");
        let app = MemoryServe::new(assets).index_file(Some("/index.html")).into_router();

        let listener = tokio::net::TcpListener::bind(endpoint).await?;
        let join_handle = spawn(async move { axum::serve(listener, app).await });
        Ok(Frontend { join_handle })
    }

    pub async fn shutdown(self) -> io::Result<()> {
        self.join_handle.await.unwrap()
    }
}