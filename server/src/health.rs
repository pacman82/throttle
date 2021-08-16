use actix_web::get;

/// Health check used to see if server is running
#[get("/health")]
async fn health() -> &'static str {
    "Ok"
}

#[cfg(test)]
mod test {
    use super::*;
    use actix_web::{http::StatusCode, test, App};

    #[actix_rt::test]
    async fn health_route() {
        let app = test::init_service(App::new().service(health)).await;
        let req = test::TestRequest::with_uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK)
    }
}
