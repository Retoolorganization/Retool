use crate::backend::{AuthSession, OidcCredentials}; // see note below on Credentials type
use crate::providers::IdentityProvider;
use crate::providers::mock::MockProvider;
use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tower_sessions::Session;

use crate::assets_store::{AssetsStore, AssetsStoreError, NewAssetInput, refresh_summary};
use crate::models::{AssetsPage, Client, Footer, FundingSummary, NextStep};

impl IntoResponse for AssetsStoreError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::MissingCredentials => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Actionstep OAuth credentials are not configured on the server.",
            ),
            Self::NotAuthenticated => (
                StatusCode::UNAUTHORIZED,
                "Sign in to Actionstep before viewing assets.",
            ),
            Self::BrowserSession(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to establish the browser session.",
            ),
            Self::InvalidConfiguration(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Actionstep OAuth configuration is invalid.",
            ),
            Self::InvalidAsset(message) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
                    .into_response();
            }
            Self::RemoteFetch(message) => {
                return (StatusCode::BAD_GATEWAY, Json(json!({ "error": message })))
                    .into_response();
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub async fn login_handler<I: IdentityProvider + 'static>(
    State(provider): State<Arc<I>>,
    session: Session,
) -> impl IntoResponse {
    let auth = provider.authorize_url();

    session.insert("csrf_token", &auth.csrf_token()).await.ok();
    session.insert("nonce", &auth.nonce()).await.ok();

    Redirect::to(&auth.url())
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

#[axum::debug_handler]
pub async fn concrete_callback(
    state: State<Arc<MockProvider>>,
    auth_session: AuthSession,
    session: Session,
    query: Query<CallbackParams>,
) -> impl IntoResponse {
    callback_handler(state, auth_session, session, query).await
}

pub async fn callback_handler<I: IdentityProvider + 'static>(
    State(provider): State<Arc<I>>,
    mut auth_session: AuthSession,
    session: Session,
    Query(params): Query<CallbackParams>,
) -> impl IntoResponse {
    // 1. Verify CSRF state
    let expected_state: Option<String> = session.get("csrf_token").await.ok().flatten();
    if expected_state.as_deref() != Some(params.state.as_str()) {
        return Redirect::to("/?error=csrf_mismatch");
    }

    // // 2. Retrieve stored nonce
    let Some(nonce): Option<String> = session.get("nonce").await.ok().flatten() else {
        return Redirect::to("/?error=missing_nonce");
    };

    // 3. Exchange code, verify ID token + nonce
    let claims = match provider.exchange_code(params.code, nonce).await {
        Ok(claims) => claims,
        Err(e) => {
            eprintln!("token exchange failed: {e:?}");
            return Redirect::to("/?error=exchange_failed");
        }
    };

    // Clean up one-time-use session values
    session.remove::<String>("csrf_token").await.ok();
    session.remove::<String>("nonce").await.ok();

    // 4. Authenticate + establish session
    let creds = OidcCredentials {
        sub: claims.sub,
        email: claims.email,
        name: claims.name,
    };

    match auth_session.authenticate(creds).await {
        Ok(Some(user)) => {
            if auth_session.login(&user).await.is_err() {
                return Redirect::to("/?error=login_failed");
            }
            Redirect::to("/protected")
        }
        _ => Redirect::to("/?error=no_user"),
    }
}

pub async fn logout_handler(mut auth_session: AuthSession) -> impl IntoResponse {
    auth_session.logout().await.ok();
    Redirect::to("/")
}

pub async fn protected_handler(auth_session: AuthSession) -> impl IntoResponse {
    match auth_session.user {
        Some(user) => format!("Hello, {}! You are logged in.", user.email),
        None => "Not logged in (you shouldn't see this).".to_string(),
    }
}

pub async fn index_handler() -> impl IntoResponse {
    "Home page. Try /login or /protected."
}

pub async fn actionstep_login_handler(
    Extension(store): Extension<AssetsStore>,
) -> Result<Redirect, AssetsStoreError> {
    let url = store.begin_login().await?;
    Ok(Redirect::temporary(&url))
}

#[derive(Deserialize)]
pub struct ActionStepCallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn actionstep_callback_handler(
    Extension(store): Extension<AssetsStore>,
    Query(params): Query<ActionStepCallbackParams>,
) -> Redirect {
    if params.error.is_some() {
        return Redirect::to(
            &store
                .frontend_redirect_url(None)
                .unwrap_or_else(|_| "/".to_string()),
        );
    }

    let result = match (params.code.as_deref(), params.state.as_deref()) {
        (Some(code), Some(state)) => store.complete_login(code, state).await,
        _ => Err(AssetsStoreError::RemoteFetch(
            "OAuth callback did not include a code and state".to_string(),
        )),
    };
    match result {
        Ok(handoff) => Redirect::to(
            &store
                .frontend_redirect_url(Some(&handoff))
                .unwrap_or_else(|_| "/".to_string()),
        ),
        Err(error) => {
            eprintln!("Actionstep OAuth callback failed: {error}");
            Redirect::to(
                &store
                    .frontend_redirect_url(None)
                    .unwrap_or_else(|_| "/".to_string()),
            )
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteActionStepLoginRequest {
    handoff_code: String,
}

pub async fn complete_actionstep_login_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
    Json(request): Json<CompleteActionStepLoginRequest>,
) -> Result<StatusCode, AssetsStoreError> {
    let session_id = store.consume_handoff(&request.handoff_code).await?;
    if let Err(error) = session.insert("actionstep_session_id", &session_id).await {
        store.logout(&session_id).await;
        return Err(AssetsStoreError::BrowserSession(error.to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn actionstep_logout_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
) -> Result<StatusCode, AssetsStoreError> {
    let session_id = session
        .get::<String>("actionstep_session_id")
        .await
        .map_err(|error| AssetsStoreError::BrowserSession(error.to_string()))?;
    if let Some(session_id) = session_id {
        store.logout(&session_id).await;
    }
    session
        .remove::<String>("actionstep_session_id")
        .await
        .map_err(|error| AssetsStoreError::BrowserSession(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn my_assets_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
) -> Result<Json<AssetsPage>, AssetsStoreError> {
    // 1. Resolve API URL and optional Bearer Token
    let session_id = session
        .get::<String>("actionstep_session_id")
        .await
        .map_err(|error| AssetsStoreError::BrowserSession(error.to_string()))?
        .ok_or(AssetsStoreError::NotAuthenticated)?;
    let mut page = default_assets_page();
    page.categories = store.get_categories(&session_id).await?;
    refresh_summary(&mut page);
    Ok(Json(page))
}

pub async fn create_asset_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
    Json(input): Json<NewAssetInput>,
) -> Result<StatusCode, AssetsStoreError> {
    let session_id = actionstep_session_id(&session).await?;
    store.create_asset(&session_id, input).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAssetRequest {
    fields: HashMap<String, String>,
}

pub async fn update_asset_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
    Path(record_id): Path<String>,
    Json(request): Json<UpdateAssetRequest>,
) -> Result<StatusCode, AssetsStoreError> {
    let session_id = actionstep_session_id(&session).await?;
    store
        .update_asset(&session_id, &record_id, request.fields)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_asset_handler(
    Extension(store): Extension<AssetsStore>,
    session: Session,
    Path(record_id): Path<String>,
) -> Result<StatusCode, AssetsStoreError> {
    let session_id = actionstep_session_id(&session).await?;
    store.delete_asset(&session_id, &record_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn actionstep_session_id(session: &Session) -> Result<String, AssetsStoreError> {
    session
        .get::<String>("actionstep_session_id")
        .await
        .map_err(|error| AssetsStoreError::BrowserSession(error.to_string()))?
        .ok_or(AssetsStoreError::NotAuthenticated)
}

fn default_assets_page() -> AssetsPage {
    AssetsPage {
        client: Client {
            name: "Client".to_string(),
            co_client: String::new(),
            report_title: "Asset Review Report".to_string(),
        },
        notice: "This report shows asset data retrieved from Actionstep. Contact your firm to request changes.".to_string(),
        funding: FundingSummary {
            percent: 0.0,
            verified: 0,
            left_to_verify: 0,
            not_fundable: 0,
            not_funded_per_client: 0,
            fundable_total: 0,
            footnote: "Percent funded excludes assets marked Not Fundable.".to_string(),
        },
        next_steps: vec![
            NextStep {
                title: "Review your assets".to_string(),
                description: "Check the details and funding status shown in your inventory.".to_string(),
            },
            NextStep {
                title: "Request an update".to_string(),
                description: "Contact your firm if any values or details need correction.".to_string(),
            },
            NextStep {
                title: "Refresh the report".to_string(),
                description: "Refresh this page to fetch the latest data from Actionstep.".to_string(),
            },
        ],
        columns: vec![
            "Asset Description".to_string(),
            "Asset Value".to_string(),
            "Funding Status".to_string(),
            "Comments".to_string(),
        ],
        categories: Vec::new(),
        total_value: 0,
        total_note: "Values with an asterisk are not included in totals.".to_string(),
        footer: Footer {
            firm: "InSight Law, PLLC".to_string(),
            address: "44365 Premier Plaza, Suite 200, Ashburn, VA 20147".to_string(),
            phone: "703-654-6019".to_string(),
            links: vec![
                "Privacy Policy".to_string(),
                "Terms of Service".to_string(),
                "Portal FAQs".to_string(),
            ],
        },
    }
}

pub async fn shubham_handler() -> impl IntoResponse {
    "Shubham page. Try /login or /protected."
}
