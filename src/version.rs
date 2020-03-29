use actix_web::get;
use version::version;

#[get("/version")]
async fn get_version() -> &'static str {
    version!()
}
