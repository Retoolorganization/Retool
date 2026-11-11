pub mod assets_store;
pub mod backend;
pub mod handlers;
pub mod models;
pub mod providers;
pub mod user;

use std::sync::Arc;

use axum::{
    Extension, Router,
    routing::{get, post, put},
};
use axum_login::{AuthManagerLayerBuilder, login_required};
use backend::MemoryBackend;
use tower_sessions::{Expiry, MemoryStore, SessionManagerLayer};

use crate::assets_store::AssetsStore;
use crate::providers::IdentityProvider;

/// Builds the full app router. Used by both `main()` and integration tests,
/// so tests exercise exactly the same wiring as production.
pub fn app<I: IdentityProvider + 'static>(
    backend: MemoryBackend,
    provider: Arc<I>,
    secure_cookies: bool,
) -> Router {
    app_with_assets_store(backend, provider, secure_cookies, AssetsStore::default())
}

pub fn app_with_assets_store<I: IdentityProvider + 'static>(
    backend: MemoryBackend,
    provider: Arc<I>,
    secure_cookies: bool,
    assets_store: AssetsStore,
) -> Router {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_expiry(Expiry::OnInactivity(time::Duration::hours(1)))
        .with_secure(secure_cookies);

    let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

    let protected = Router::new()
        .route("/protected", get(handlers::protected_handler))
        .route_layer(login_required!(MemoryBackend, login_url = "/"));

    let router = Router::new()
        .merge(protected)
        .route("/", get(handlers::index_handler))
        .route("/login", get(handlers::login_handler))
        .route("/callback", get(handlers::callback_handler))
        .route("/logout", get(handlers::logout_handler))
        .route("/api/auth/login", get(handlers::actionstep_login_handler))
        .route(
            "/api/auth/complete",
            post(handlers::complete_actionstep_login_handler),
        )
        .route(
            "/api/auth/logout",
            post(handlers::actionstep_logout_handler),
        )
        .route(
            "/api/assets",
            get(handlers::my_assets_handler).post(handlers::create_asset_handler),
        )
        .route(
            "/api/assets/{record_id}",
            put(handlers::update_asset_handler).delete(handlers::delete_asset_handler),
        )
        .route("/shubham", get(handlers::shubham_handler))
        .layer(Extension(assets_store))
        .layer(auth_layer);

    router.with_state(provider)
}

pub fn actionstep_callback_app(assets_store: AssetsStore) -> Router {
    Router::new()
        .route("/", get(handlers::actionstep_callback_handler))
        .layer(Extension(assets_store))
}

#[cfg(test)]
mod tests {
    use crate::providers::mock::MockProvider;

    use super::*;
    use axum::http::StatusCode;
    use axum_test::{TestServer, TestServerConfig};

    fn test_server() -> TestServer {
        let router = app(
            MemoryBackend::default(),
            Arc::new(MockProvider::default()),
            false,
        );
        let config = TestServerConfig {
            save_cookies: true,
            ..Default::default()
        };
        TestServer::new_with_config(router, config)
    }

    #[tokio::test]
    async fn debug_protected_redirect_status() {
        let server = test_server();
        let response = server.get("/protected").await;
        println!("status: {:?}", response.status_code());
        println!("location: {:?}", response.header("location"));
    }

    #[tokio::test]
    async fn protected_redirects_when_logged_out() {
        let server = test_server();
        let response = server.get("/protected").await;

        response.assert_status(StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.header("location"), "/?next=%2Fprotected");
    }

    #[tokio::test]
    async fn login_redirects_to_provider_auth_url() {
        let server = test_server();

        let response = server.get("/login").await;

        response.assert_status_see_other(); // or whatever status Redirect::to() issues — check if this still passes
        let binding = response.header("location");
        let location = binding.to_str().unwrap();
        assert!(location.starts_with("https://mock-provider.test/authorize"));
    }

    #[tokio::test]
    async fn protected_succeeds_after_full_login_flow() {
        let server = test_server();

        server.get("/login").await; // establishes csrf_token/nonce in session

        let response = server
            .get("/callback?code=fake-code&state=fake-csrf-token")
            .await;
        response.assert_status_see_other();
        assert_eq!(response.header("location"), "/protected");

        let protected = server.get("/protected").await;
        protected.assert_status_ok();
        protected.assert_text_contains("mock@example.com"); // MockProvider's default email, not "test@example.com"
    }

    #[tokio::test]
    async fn debug_callback_response() {
        let server = test_server();
        server.get("/login").await;

        let response = server
            .get("/callback?code=fake-code&state=fake-csrf-token")
            .await;

        println!("status: {:?}", response.status_code());
        println!("location: {:?}", response.header("location"));
        println!("body: {:?}", response.text());
    }

    #[tokio::test]
    async fn logout_clears_session() {
        let server = test_server();
        server.get("/login").await;
        server.get("/logout").await;

        let response = server.get("/protected").await;

        response.assert_status(StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.header("location"), "/?next=%2Fprotected");
    }

    #[tokio::test]
    async fn index_is_publicly_accessible() {
        let server = test_server();

        let response = server.get("/").await;

        response.assert_status_ok();
    }

    #[tokio::test]
    async fn asset_mutations_require_an_actionstep_session() {
        let server = test_server();
        server
            .post("/api/assets")
            .json(&serde_json::json!({
                "category": "01 - Cash Account",
                "description": "Checking",
                "assetType": "Checking",
                "value": "1000"
            }))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        server
            .put("/api/assets/2819")
            .json(&serde_json::json!({"fields": {"Asset_Value": "12500"}}))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        server
            .delete("/api/assets/2819")
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }
}
