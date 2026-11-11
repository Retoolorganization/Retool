use ::backend::{
    actionstep_callback_app, app_with_assets_store, assets_store::AssetsStore,
    backend::MemoryBackend, providers::mock::MockProvider,
};

mod assets_store;
mod backend;
mod handlers;
mod models;
mod providers;
mod user;

use std::{env, sync::Arc};

fn is_production() -> bool {
    match env::var("APP_ENV").as_deref() {
        Ok("development") => false,
        Ok("production") | Err(_) => true, // unset = production, safe default
        Ok(other) => panic!("APP_ENV must be 'production' or 'development', got '{other}'"),
    }
}

// #[tokio::main]
// async fn main() -> std::io::Result<()> {
//     // unsafe {
//     //     std::env::set_var("RUST_LOG", "axum=debug");
//     // }
//     // build our application with a single route
//     let app = Router::new()
//         .route("/", get(|| async { "Hello, World!" }))
//         .route("/users", get(handlers::get_users))
//         .route("/users/{id}", get(handlers::get_user_by_id))
//         .route("/users", post(handlers::add_user))
//         .route("/users/{id}", delete(handlers::delete_user));

//     // run our app with hyper, listening globally on port 3000
//     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//     axum::serve(listener, app).await
// }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let production = is_production();
    println!(
        "Starting in {} mode (secure cookies: {})",
        if production {
            "production"
        } else {
            "development"
        },
        production
    );

    let assets_store = AssetsStore::default();
    let callback_listener =
        tokio::net::TcpListener::bind(assets_store.callback_bind_address()?).await?;
    let callback_router = actionstep_callback_app(assets_store.clone());
    tokio::spawn(async move {
        if let Err(error) = axum::serve(callback_listener, callback_router).await {
            eprintln!("Actionstep OAuth callback server failed: {error}");
        }
    });

    let router = app_with_assets_store(
        MemoryBackend::default(),
        Arc::new(MockProvider::default()),
        production,
        assets_store,
    );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, router).await?;
    Ok(())
}
