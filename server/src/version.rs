use version::version;

pub async fn version() -> &'static str {
    version!()
}
