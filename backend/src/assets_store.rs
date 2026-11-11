use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use crate::models::{Asset, AssetCategory, AssetStatus, AssetsPage};
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

const DEFAULT_ASSET_API_URL: &str = "https://us-west-2.actionstepstaging.com/api/rest/datacollectionrecordvalues?action=7546&dataCollection=105&filter=stringValue+IS+NOT+NULL&dataCollectionField[name_in]=Asset_Category,FinancialAdvisor,FNDCompany,Type,CustomType,Asset_Description,ReportAcctNo,Original_Owner,TypeOfOwner,LinkedOriginalOwner,CustomOrigOwner,New_Owner,LinkedNewOwner,CustomNewOwner,PBene,LinkedPBene,CustomBene,CBene,LinkedCBene,CustomContBene,InsuredLife,Insured-Annuitant,Asset_Value,Cash_Value,LienAmount,Equity_Basis,Funding_Status,Comments_for_Client,Deleted,Client_Added,ValueTo,SortOrder&include=dataCollectionField&fields[datacollectionfields]=name,dataType,label,formOrder,description&pageSize=200&page=1";
const DEFAULT_AUTHORITY: &str = "https://go.actionstepstaging.com";
const DEFAULT_TOKEN_ENDPOINT: &str = "https://api.actionstepstaging.com/api/oauth/token";
const DEFAULT_FRONTEND_URL: &str = "http://localhost:5173/";
const MAX_API_PAGES: usize = 20;
const EDITABLE_ASSET_FIELDS: &[&str] = &[
    "Asset_Value",
    "Asset_Description",
    "ReportAcctNo",
    "Type",
    "Comments_for_Client",
    "LienAmount",
    "Equity_Basis",
];

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAssetInput {
    pub category: String,
    pub description: String,
    #[serde(default)]
    pub report_acct_no: String,
    pub asset_type: String,
    pub value: String,
    #[serde(default)]
    pub comments: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AssetsStoreError {
    #[error("Actionstep OAuth credentials are not configured")]
    MissingCredentials,
    #[error("Actionstep sign-in is required")]
    NotAuthenticated,
    #[error("Unable to manage the Actionstep browser session: {0}")]
    BrowserSession(String),
    #[error("Invalid Actionstep OAuth configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Invalid asset data: {0}")]
    InvalidAsset(String),
    #[error("Unable to load Actionstep assets: {0}")]
    RemoteFetch(String),
}

#[derive(Debug, Clone)]
struct ActionStepSession {
    access_token: String,
    refresh_token: String,
    expires_at: std::time::SystemTime,
    api_endpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AssetsStore {
    api_url: String,
    authority: String,
    token_endpoint: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    scopes: String,
    redirect_host: String,
    redirect_port: u16,
    frontend_url: String,
    sessions: Arc<Mutex<HashMap<String, ActionStepSession>>>,
    handoffs: Arc<Mutex<HashMap<String, String>>>,
    oauth_states: Arc<Mutex<HashSet<String>>>,
}

impl Default for AssetsStore {
    fn default() -> Self {
        Self {
            api_url: std::env::var("ASSET_API_URL")
                .unwrap_or_else(|_| DEFAULT_ASSET_API_URL.to_string()),
            authority: std::env::var("ACTIONSTEP_AUTHORITY")
                .unwrap_or_else(|_| DEFAULT_AUTHORITY.to_string()),
            token_endpoint: std::env::var("ACTIONSTEP_TOKEN_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_TOKEN_ENDPOINT.to_string()),
            client_id: std::env::var("ACTIONSTEP_CLIENT_ID").ok(),
            client_secret: std::env::var("ACTIONSTEP_CLIENT_SECRET").ok(),
            scopes: std::env::var("ACTIONSTEP_SCOPE").unwrap_or_else(|_| "all".to_string()),
            redirect_host: std::env::var("ACTIONSTEP_REDIRECT_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            redirect_port: std::env::var("ACTIONSTEP_REDIRECT_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(7890),
            frontend_url: std::env::var("ACTIONSTEP_FRONTEND_URL")
                .unwrap_or_else(|_| DEFAULT_FRONTEND_URL.to_string()),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            handoffs: Arc::new(Mutex::new(HashMap::new())),
            oauth_states: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl AssetsStore {
    pub fn with_oauth_config(
        api_url: impl Into<String>,
        token_endpoint: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            api_url: api_url.into(),
            authority: DEFAULT_AUTHORITY.to_string(),
            token_endpoint: token_endpoint.into(),
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            scopes: "all".to_string(),
            redirect_host: "127.0.0.1".to_string(),
            redirect_port: 7890,
            frontend_url: DEFAULT_FRONTEND_URL.to_string(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            handoffs: Arc::new(Mutex::new(HashMap::new())),
            oauth_states: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn credentials(&self) -> Result<(&str, &str), AssetsStoreError> {
        let client_id = self
            .client_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(AssetsStoreError::MissingCredentials)?;
        let client_secret = self
            .client_secret
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(AssetsStoreError::MissingCredentials)?;
        Ok((client_id.trim(), client_secret.trim()))
    }

    pub fn redirect_uri(&self) -> Result<String, AssetsStoreError> {
        let ip = self.redirect_host.parse::<IpAddr>().map_err(|_| {
            AssetsStoreError::InvalidConfiguration(
                "callback host must be a loopback IP address".to_string(),
            )
        })?;
        if !ip.is_loopback() {
            return Err(AssetsStoreError::InvalidConfiguration(
                "callback host must be a loopback IP address".to_string(),
            ));
        }
        Ok(format!("http://{}:{}/", ip, self.redirect_port))
    }

    pub fn callback_bind_address(&self) -> Result<SocketAddr, AssetsStoreError> {
        let ip = self.redirect_host.parse::<IpAddr>().map_err(|_| {
            AssetsStoreError::InvalidConfiguration(
                "callback host must be a loopback IP address".to_string(),
            )
        })?;
        if !ip.is_loopback() {
            return Err(AssetsStoreError::InvalidConfiguration(
                "callback host must be a loopback IP address".to_string(),
            ));
        }
        Ok(SocketAddr::new(ip, self.redirect_port))
    }

    pub async fn begin_login(&self) -> Result<String, AssetsStoreError> {
        let (client_id, _) = self.credentials()?;
        let authority = reqwest::Url::parse(&self.authority)
            .map_err(|error| AssetsStoreError::InvalidConfiguration(error.to_string()))?;
        if authority.scheme() != "https" || authority.host_str().is_none() {
            return Err(AssetsStoreError::InvalidConfiguration(
                "authorization authority must be an HTTPS URL".to_string(),
            ));
        }
        let redirect_uri = self.redirect_uri()?;
        let mut authorization_url = authority
            .join("/api/oauth/authorize")
            .map_err(|error| AssetsStoreError::InvalidConfiguration(error.to_string()))?;
        let state = Uuid::new_v4().to_string();
        authorization_url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", self.scopes.trim())
            .append_pair("state", &state);
        self.oauth_states.lock().await.insert(state);
        Ok(authorization_url.to_string())
    }

    pub async fn complete_login(
        &self,
        code: &str,
        state: &str,
    ) -> Result<String, AssetsStoreError> {
        if !self.oauth_states.lock().await.remove(state) {
            return Err(AssetsStoreError::RemoteFetch(
                "OAuth state validation failed".to_string(),
            ));
        }
        let redirect_uri = self.redirect_uri()?;
        let token = self
            .request_token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &redirect_uri),
            ])
            .await?;
        let session_id = Uuid::new_v4().to_string();
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), token.into_session(None, None)?);
        let handoff = Uuid::new_v4().to_string();
        self.handoffs
            .lock()
            .await
            .insert(handoff.clone(), session_id);
        Ok(handoff)
    }

    pub async fn consume_handoff(&self, handoff: &str) -> Result<String, AssetsStoreError> {
        self.handoffs
            .lock()
            .await
            .remove(handoff)
            .ok_or(AssetsStoreError::NotAuthenticated)
    }

    pub async fn logout(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }

    pub fn frontend_redirect_url(&self, handoff: Option<&str>) -> Result<String, AssetsStoreError> {
        let mut frontend_url = reqwest::Url::parse(&self.frontend_url)
            .map_err(|error| AssetsStoreError::InvalidConfiguration(error.to_string()))?;
        if !matches!(frontend_url.scheme(), "http" | "https")
            || frontend_url.host_str().is_none()
            || !frontend_url.username().is_empty()
            || frontend_url.password().is_some()
        {
            return Err(AssetsStoreError::InvalidConfiguration(
                "frontend URL must be an HTTP(S) URL".to_string(),
            ));
        }
        let fragment = match handoff {
            Some(code) => format!("actionstep_handoff={code}"),
            None => "actionstep_error=1".to_string(),
        };
        frontend_url.set_fragment(Some(&fragment));
        Ok(frontend_url.to_string())
    }

    async fn request_token(
        &self,
        form: &[(&str, &str)],
    ) -> Result<ActionStepTokenResponse, AssetsStoreError> {
        let (client_id, client_secret) = self.credentials()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        let response = client
            .post(&self.token_endpoint)
            .basic_auth(client_id, Some(client_secret))
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AssetsStoreError::RemoteFetch(format!(
                "OAuth token endpoint returned HTTP {}",
                response.status()
            )));
        }
        let token: ActionStepTokenResponse = response
            .json()
            .await
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        if token.access_token.trim().is_empty() {
            return Err(AssetsStoreError::RemoteFetch(
                "OAuth response did not include an access token".to_string(),
            ));
        }
        Ok(token)
    }

    async fn access_token(
        &self,
        session_id: &str,
    ) -> Result<(String, Option<String>), AssetsStoreError> {
        let mut sessions = self.sessions.lock().await;
        let current = sessions
            .get(session_id)
            .ok_or(AssetsStoreError::NotAuthenticated)?;
        if current.expires_at > std::time::SystemTime::now() + Duration::from_secs(60) {
            return Ok((current.access_token.clone(), current.api_endpoint.clone()));
        }

        let refresh_token = current.refresh_token.clone();
        let refreshed = self
            .request_token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
            ])
            .await;
        match refreshed {
            Ok(token) => {
                let refreshed_session = token.into_session(
                    Some(refresh_token),
                    sessions
                        .get(session_id)
                        .and_then(|s| s.api_endpoint.clone()),
                )?;
                let result = (
                    refreshed_session.access_token.clone(),
                    refreshed_session.api_endpoint.clone(),
                );
                sessions.insert(session_id.to_string(), refreshed_session);
                Ok(result)
            }
            Err(error) => {
                sessions.remove(session_id);
                Err(match error {
                    AssetsStoreError::RemoteFetch(_) => AssetsStoreError::NotAuthenticated,
                    other => other,
                })
            }
        }
    }

    fn assets_url(&self, api_endpoint: Option<&str>) -> Result<reqwest::Url, AssetsStoreError> {
        let mut asset_url = reqwest::Url::parse(&self.api_url)
            .map_err(|error| AssetsStoreError::InvalidConfiguration(error.to_string()))?;
        if let Some(api_endpoint) = api_endpoint.filter(|value| !value.trim().is_empty()) {
            let api_endpoint = reqwest::Url::parse(api_endpoint)
                .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
            let is_loopback = api_endpoint
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback());
            if (api_endpoint.scheme() != "https"
                && !(api_endpoint.scheme() == "http" && is_loopback))
                || api_endpoint.host_str().is_none()
                || !api_endpoint.username().is_empty()
                || api_endpoint.password().is_some()
            {
                return Err(AssetsStoreError::RemoteFetch(
                    "OAuth response contained an invalid API endpoint".to_string(),
                ));
            }
            asset_url.set_scheme(api_endpoint.scheme()).map_err(|_| {
                AssetsStoreError::RemoteFetch("invalid API endpoint scheme".to_string())
            })?;
            asset_url.set_host(api_endpoint.host_str()).map_err(|_| {
                AssetsStoreError::RemoteFetch("invalid API endpoint host".to_string())
            })?;
            asset_url.set_port(api_endpoint.port()).map_err(|_| {
                AssetsStoreError::RemoteFetch("invalid API endpoint port".to_string())
            })?;
        }
        let is_loopback = asset_url
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if (asset_url.scheme() != "https" && !(asset_url.scheme() == "http" && is_loopback))
            || asset_url.host_str().is_none()
        {
            return Err(AssetsStoreError::InvalidConfiguration(
                "assets endpoint must be an HTTPS URL".to_string(),
            ));
        }
        Ok(asset_url)
    }

    fn actionstep_ids(&self) -> Result<(String, String), AssetsStoreError> {
        let url = reqwest::Url::parse(&self.api_url)
            .map_err(|error| AssetsStoreError::InvalidConfiguration(error.to_string()))?;
        let mut action = None;
        let mut collection = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "action" => action = Some(value.into_owned()),
                "dataCollection" => collection = Some(value.into_owned()),
                _ => {}
            }
        }
        let action = action.filter(|value| !value.is_empty()).ok_or_else(|| {
            AssetsStoreError::InvalidConfiguration(
                "asset endpoint must specify an Actionstep action".to_string(),
            )
        })?;
        let collection = collection
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AssetsStoreError::InvalidConfiguration(
                    "asset endpoint must specify an Actionstep data collection".to_string(),
                )
            })?;
        if !action.chars().all(|c| c.is_ascii_digit())
            || !collection.chars().all(|c| c.is_ascii_digit())
        {
            return Err(AssetsStoreError::InvalidConfiguration(
                "Actionstep action and data collection identifiers must be numeric".to_string(),
            ));
        }
        Ok((action, collection))
    }

    fn resource_url(
        &self,
        api_endpoint: Option<&str>,
        path: &str,
    ) -> Result<reqwest::Url, AssetsStoreError> {
        let mut url = self.assets_url(api_endpoint)?;
        url.set_path(path);
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    fn http_client() -> Result<reqwest::Client, AssetsStoreError> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))
    }

    pub async fn create_asset(
        &self,
        session_id: &str,
        input: NewAssetInput,
    ) -> Result<(), AssetsStoreError> {
        let category = required_asset_value(input.category, "category")?;
        let description = required_asset_value(input.description, "description")?;
        let asset_type = required_asset_value(input.asset_type, "type")?;
        let value = validate_asset_value(&input.value, "Asset_Value")?;
        let account_number = validate_optional_text(input.report_acct_no, "ReportAcctNo")?;
        let comments = validate_optional_text(input.comments, "Comments_for_Client")?;
        let (action, collection) = self.actionstep_ids()?;
        let (access_token, api_endpoint) = self.access_token(session_id).await?;
        let client = Self::http_client()?;
        let create_url =
            self.resource_url(api_endpoint.as_deref(), "/api/rest/datacollectionrecords")?;
        let response = client
            .post(create_url)
            .bearer_auth(&access_token)
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.api+json, application/json",
            )
            .json(&serde_json::json!({
                "datacollectionrecords": {
                    "links": {
                        "action": action,
                        "dataCollection": collection
                    }
                }
            }))
            .send()
            .await
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AssetsStoreError::RemoteFetch(format!(
                "Actionstep asset record creation returned HTTP {}",
                response.status()
            )));
        }
        let response: serde_json::Value = response
            .json()
            .await
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        let record_id = response
            .pointer("/datacollectionrecords/id")
            .and_then(|id| {
                id.as_str()
                    .map(str::to_string)
                    .or_else(|| id.as_u64().map(|n| n.to_string()))
            })
            .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
            .ok_or_else(|| {
                AssetsStoreError::RemoteFetch(
                    "Actionstep did not return a valid asset record ID".to_string(),
                )
            })?;
        let values = vec![
            ("Asset_Category".to_string(), category),
            ("Asset_Description".to_string(), description),
            ("ReportAcctNo".to_string(), account_number),
            ("Type".to_string(), asset_type),
            ("Asset_Value".to_string(), value),
            (
                "Comments_for_Client".to_string(),
                if comments.is_empty() {
                    "Client submitted through portal".to_string()
                } else {
                    comments
                },
            ),
            ("Client_Added".to_string(), "T".to_string()),
        ];
        if let Err(error) = self
            .put_record_values(
                &client,
                &access_token,
                api_endpoint.as_deref(),
                &action,
                &collection,
                &record_id,
                values,
            )
            .await
        {
            let cleanup = self
                .put_record_values(
                    &client,
                    &access_token,
                    api_endpoint.as_deref(),
                    &action,
                    &collection,
                    &record_id,
                    vec![("Deleted".to_string(), "DELETED".to_string())],
                )
                .await;
            let cleanup_detail = match cleanup {
                Ok(()) => "the incomplete record was marked DELETED".to_string(),
                Err(cleanup_error) => {
                    format!("marking the incomplete record DELETED also failed: {cleanup_error}")
                }
            };
            return Err(AssetsStoreError::RemoteFetch(format!(
                "Asset record was created but could not be populated: {error}; {cleanup_detail}"
            )));
        }
        Ok(())
    }

    pub async fn update_asset(
        &self,
        session_id: &str,
        record_id: &str,
        fields: HashMap<String, String>,
    ) -> Result<(), AssetsStoreError> {
        validate_record_id(record_id)?;
        if fields.is_empty() {
            return Err(AssetsStoreError::InvalidAsset(
                "provide at least one editable field".to_string(),
            ));
        }
        let values = fields
            .into_iter()
            .map(|(field, value)| {
                if !EDITABLE_ASSET_FIELDS.contains(&field.as_str()) {
                    return Err(AssetsStoreError::InvalidAsset(format!(
                        "{field} cannot be changed through the portal"
                    )));
                }
                let value = if matches!(
                    field.as_str(),
                    "Asset_Value" | "LienAmount" | "Equity_Basis"
                ) {
                    if field != "Asset_Value" && value.trim().is_empty() {
                        String::new()
                    } else {
                        validate_asset_value(&value, &field)?
                    }
                } else {
                    validate_optional_text(value, &field)?
                };
                Ok((field, value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (action, collection) = self.actionstep_ids()?;
        let (access_token, api_endpoint) = self.access_token(session_id).await?;
        self.put_record_values(
            &Self::http_client()?,
            &access_token,
            api_endpoint.as_deref(),
            &action,
            &collection,
            record_id,
            values,
        )
        .await
    }

    pub async fn delete_asset(
        &self,
        session_id: &str,
        record_id: &str,
    ) -> Result<(), AssetsStoreError> {
        validate_record_id(record_id)?;
        let (action, collection) = self.actionstep_ids()?;
        let (access_token, api_endpoint) = self.access_token(session_id).await?;
        self.put_record_values(
            &Self::http_client()?,
            &access_token,
            api_endpoint.as_deref(),
            &action,
            &collection,
            record_id,
            vec![("Deleted".to_string(), "DELETED".to_string())],
        )
        .await
    }

    async fn put_record_values(
        &self,
        client: &reqwest::Client,
        access_token: &str,
        api_endpoint: Option<&str>,
        action: &str,
        collection: &str,
        record_id: &str,
        values: Vec<(String, String)>,
    ) -> Result<(), AssetsStoreError> {
        validate_record_id(record_id)?;
        let entries = values
            .into_iter()
            .map(|(field, value)| ActionStepRecordValue {
                id: format!("{collection}--{field}--{record_id}"),
                string_value: value,
                links: ActionStepRecordLinks {
                    action: action.to_string(),
                    data_collection_field: format!("{collection}--{field}"),
                    data_collection_record: record_id.to_string(),
                    data_collection: collection.to_string(),
                },
            })
            .collect::<Vec<_>>();
        let ids = entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let url = self.resource_url(
            api_endpoint,
            &format!("/api/rest/datacollectionrecordvalues/{ids}"),
        )?;
        let response = client
            .put(url)
            .bearer_auth(access_token)
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.api+json, application/json",
            )
            .json(&serde_json::json!({"datacollectionrecordvalues": entries}))
            .send()
            .await
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AssetsStoreError::RemoteFetch(format!(
                "Actionstep asset update returned HTTP {}",
                response.status()
            )));
        }
        Ok(())
    }

    pub async fn get_categories(
        &self,
        session_id: &str,
    ) -> Result<Vec<AssetCategory>, AssetsStoreError> {
        let (access_token, api_endpoint) = self.access_token(session_id).await?;
        let initial_url = self.assets_url(api_endpoint.as_deref())?;
        let origin = initial_url.origin().ascii_serialization();
        let mut current_url = Some(initial_url);
        let mut all_records = Vec::new();
        let mut seen_urls = HashSet::new();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;

        while let Some(url) = current_url.take() {
            if seen_urls.len() >= MAX_API_PAGES {
                return Err(AssetsStoreError::RemoteFetch(
                    "assets API exceeded the pagination limit".to_string(),
                ));
            }
            if url.origin().ascii_serialization() != origin
                || !seen_urls.insert(url.as_str().to_string())
            {
                return Err(AssetsStoreError::RemoteFetch(
                    "assets API returned an invalid or repeated pagination URL".to_string(),
                ));
            }
            let response = client
                .get(url.clone())
                .header(
                    reqwest::header::ACCEPT,
                    "application/vnd.api+json, application/json",
                )
                .bearer_auth(&access_token)
                .send()
                .await
                .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
            if !response.status().is_success() {
                return Err(AssetsStoreError::RemoteFetch(format!(
                    "assets API returned HTTP {}",
                    response.status()
                )));
            }
            let payload: ActionStepAssetResponse = response
                .json()
                .await
                .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?;
            let next_page = payload
                .meta
                .and_then(|meta| meta.paging)
                .and_then(|paging| paging.datacollectionrecordvalues)
                .and_then(|details| details.next_page)
                .filter(|next| !next.trim().is_empty());
            current_url = next_page
                .map(|next| next_page_url(&url, &next, &origin))
                .transpose()?;
            all_records.extend(payload.datacollectionrecordvalues);
        }
        Ok(build_categories_from_remote_records(&all_records))
    }
}

#[derive(Serialize)]
struct ActionStepRecordValue {
    id: String,
    #[serde(rename = "stringValue")]
    string_value: String,
    links: ActionStepRecordLinks,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionStepRecordLinks {
    action: String,
    data_collection_field: String,
    data_collection_record: String,
    data_collection: String,
}

fn validate_record_id(record_id: &str) -> Result<(), AssetsStoreError> {
    if record_id.is_empty() || !record_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(AssetsStoreError::InvalidAsset(
            "asset record ID must be numeric".to_string(),
        ));
    }
    Ok(())
}

fn validate_optional_text(value: String, field: &str) -> Result<String, AssetsStoreError> {
    let value = value.trim().to_string();
    if value.chars().count() > 2000 {
        return Err(AssetsStoreError::InvalidAsset(format!(
            "{field} must be 2000 characters or fewer"
        )));
    }
    Ok(value)
}

fn required_asset_value(value: String, field: &str) -> Result<String, AssetsStoreError> {
    let value = validate_optional_text(value, field)?;
    if value.is_empty() {
        return Err(AssetsStoreError::InvalidAsset(format!(
            "{field} is required"
        )));
    }
    Ok(value)
}

fn validate_asset_value(value: &str, field: &str) -> Result<String, AssetsStoreError> {
    let value = value.trim();
    let parsed = value
        .parse::<f64>()
        .map_err(|_| AssetsStoreError::InvalidAsset(format!("{field} must be a valid number")))?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(AssetsStoreError::InvalidAsset(format!(
            "{field} must be a non-negative finite number"
        )));
    }
    Ok(value.to_string())
}

#[derive(Debug, serde::Deserialize)]
struct ActionStepTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    api_endpoint: Option<String>,
}

impl ActionStepTokenResponse {
    fn into_session(
        self,
        previous_refresh_token: Option<String>,
        previous_api_endpoint: Option<String>,
    ) -> Result<ActionStepSession, AssetsStoreError> {
        let refresh_token = self
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .or(previous_refresh_token)
            .ok_or_else(|| {
                AssetsStoreError::RemoteFetch(
                    "OAuth response did not include a refresh token".to_string(),
                )
            })?;
        let expires_in = self.expires_in.unwrap_or(3600).saturating_sub(30);
        Ok(ActionStepSession {
            access_token: self.access_token,
            refresh_token,
            expires_at: std::time::SystemTime::now() + Duration::from_secs(expires_in),
            api_endpoint: self.api_endpoint.or(previous_api_endpoint),
        })
    }
}

fn next_page_url(
    current_url: &reqwest::Url,
    next_page: &str,
    expected_origin: &str,
) -> Result<reqwest::Url, AssetsStoreError> {
    let next_page = next_page.trim();
    let mut next_url = if let Ok(page_number) = next_page.parse::<u32>() {
        if page_number < 1 {
            return Err(AssetsStoreError::RemoteFetch(
                "assets API returned an invalid next page".to_string(),
            ));
        }
        let mut url = current_url.clone();
        let mut query = url
            .query_pairs()
            .filter(|(key, _)| key != "page")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        query.push(("page".to_string(), page_number.to_string()));
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(query);
        url
    } else {
        current_url
            .join(next_page)
            .map_err(|error| AssetsStoreError::RemoteFetch(error.to_string()))?
    };
    if next_url.origin().ascii_serialization() != expected_origin {
        return Err(AssetsStoreError::RemoteFetch(
            "assets API returned a pagination URL on a different origin".to_string(),
        ));
    }
    next_url.set_fragment(None);
    Ok(next_url)
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepAssetResponse {
    #[serde(default)]
    pub datacollectionrecordvalues: Vec<ActionStepRecord>,
    #[serde(default)]
    pub meta: Option<ActionStepMeta>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepMeta {
    #[serde(default)]
    pub paging: Option<ActionStepPaging>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepPaging {
    #[serde(default)]
    pub datacollectionrecordvalues: Option<ActionStepPagingDetails>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepPagingDetails {
    #[serde(rename = "nextPage", default)]
    pub next_page: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepRecord {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "stringValue", default)]
    pub string_value: Option<String>,
    #[serde(rename = "numericValue", default)]
    pub numeric_value: Option<serde_json::Value>,
    #[serde(default)]
    pub links: Option<ActionStepLinks>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ActionStepLinks {
    #[serde(rename = "dataCollectionField", default)]
    pub data_collection_field: String,
    #[serde(rename = "dataCollectionRecord", default)]
    pub data_collection_record: String,
}

pub fn build_categories_from_remote_records(records: &[ActionStepRecord]) -> Vec<AssetCategory> {
    let mut by_record: HashMap<String, HashMap<String, String>> = HashMap::new();

    for record in records {
        let Some(links) = &record.links else {
            continue;
        };
        let field_name = field_name_from_collection_value(&links.data_collection_field);
        if field_name.is_empty() {
            continue;
        }

        let record_id = if links.data_collection_record.trim().is_empty() {
            record.id.clone()
        } else {
            links.data_collection_record.clone()
        };

        let value = record
            .string_value
            .clone()
            .or_else(|| {
                record.numeric_value.as_ref().map(|v| match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
            })
            .unwrap_or_default();

        if value.trim().is_empty() {
            continue;
        }

        by_record
            .entry(record_id)
            .or_default()
            .insert(field_name, value);
    }

    struct CategoryBucket {
        order: usize,
        id: String,
        name: String,
        icon: String,
        assets: Vec<Asset>,
    }

    let mut buckets: HashMap<String, CategoryBucket> = HashMap::new();

    // Sort record IDs for deterministic order
    let mut record_ids: Vec<String> = by_record.keys().cloned().collect();
    record_ids.sort_by(|a, b| {
        let a_num = a.parse::<u64>().ok();
        let b_num = b.parse::<u64>().ok();
        match (a_num, b_num) {
            (Some(an), Some(bn)) => an.cmp(&bn),
            _ => a.cmp(b),
        }
    });

    for record_id in record_ids {
        let fields = &by_record[&record_id];

        // 1. Exclude deleted assets
        if let Some(deleted) = fields.get("Deleted") {
            if deleted.trim().eq_ignore_ascii_case("deleted") {
                continue;
            }
        }

        let category_raw = fields
            .get("Asset_Category")
            .or_else(|| fields.get("Asset Category"))
            .cloned()
            .unwrap_or_else(|| "17 - Other Asset".to_string());

        let (category_id, category_name, category_icon, category_order) =
            category_meta_for(&category_raw);

        let company = fields.get("FNDCompany").cloned();
        let type_desc = fields
            .get("CustomType")
            .or_else(|| fields.get("Type"))
            .cloned();
        let acct_desc = fields
            .get("Asset_Description")
            .or_else(|| fields.get("Asset Description"))
            .cloned()
            .unwrap_or_default();
        let report_acct = fields
            .get("ReportAcctNo")
            .or_else(|| fields.get("Report Acct No"))
            .cloned();

        let is_desc_acct = is_account_number_str(&acct_desc);

        // 2. Build asset name
        let name = if let Some(ref comp) = company {
            if let Some(ref t) = type_desc {
                format!("{comp} – {t}")
            } else if !acct_desc.is_empty() && !is_desc_acct {
                format!("{comp} – {acct_desc}")
            } else {
                comp.clone()
            }
        } else if !acct_desc.is_empty() && !is_desc_acct {
            acct_desc.clone()
        } else if let Some(ref t) = type_desc {
            t.clone()
        } else {
            singular_category_name(&category_name).to_string()
        };

        // 3. Build details lines
        let mut detail_lines: Vec<String> = Vec::new();

        let acct_mask = report_acct.or_else(|| {
            if is_desc_acct && !acct_desc.is_empty() {
                Some(format_masked_acct(&acct_desc))
            } else {
                None
            }
        });

        if let Some(acct) = acct_mask {
            if !acct.trim().is_empty() {
                detail_lines.push(acct);
            }
        }

        let orig_owner = fields
            .get("CustomOrigOwner")
            .or_else(|| fields.get("Original_Owner"))
            .or_else(|| fields.get("LinkedOriginalOwner"));
        if let Some(owner) = orig_owner {
            let clean = owner.replace("\r\n", " ").replace('\n', " ");
            let clean = clean.trim();
            if !clean.is_empty() && clean != "N/A" {
                detail_lines.push(format!("Original Owner: {clean}"));
            }
        }

        let new_owner = fields
            .get("CustomNewOwner")
            .or_else(|| fields.get("New_Owner"))
            .or_else(|| fields.get("LinkedNewOwner"));
        if let Some(owner) = new_owner {
            let clean = owner.replace("\r\n", " ").replace('\n', " ");
            let clean = clean.trim();
            if !clean.is_empty() && clean != "N/A" {
                detail_lines.push(format!("New Owner: {clean}"));
            }
        }

        let insured = fields
            .get("InsuredLife")
            .or_else(|| fields.get("Insured-Annuitant"));
        if let Some(ins) = insured {
            let clean = ins.trim();
            if !clean.is_empty() && clean != "N/A" {
                detail_lines.push(format!("Insured: {clean}"));
            }
        }

        let pbene = fields
            .get("CustomBene")
            .or_else(|| fields.get("PBene"))
            .or_else(|| fields.get("LinkedPBene"));
        if let Some(b) = pbene {
            let clean = b.replace("\r\n", " ").replace('\n', " ");
            let clean = clean.trim();
            if !clean.is_empty() && clean != "N/A" {
                detail_lines.push(format!("Primary Beneficiary: {clean}"));
            }
        }

        let cbene = fields
            .get("CustomContBene")
            .or_else(|| fields.get("CBene"))
            .or_else(|| fields.get("LinkedCBene"));
        if let Some(b) = cbene {
            let clean = b.replace("\r\n", " ").replace('\n', " ");
            let clean = clean.trim();
            if !clean.is_empty() && clean != "N/A" {
                detail_lines.push(format!("Contingent Beneficiary: {clean}"));
            }
        }

        let details = detail_lines.join("\n");

        // 4. Value
        let value = parse_asset_value(
            fields
                .get("Asset_Value")
                .or_else(|| fields.get("Cash_Value"))
                .or_else(|| fields.get("Equity_Basis"))
                .map(String::as_str)
                .unwrap_or("0"),
        );

        // 5. Funding status
        let (status, status_label) = parse_asset_funding_status(
            fields
                .get("Funding_Status")
                .map(String::as_str)
                .unwrap_or("Undecided"),
        );

        // 6. Notes / Comments
        let note = fields
            .get("Comments_for_Client")
            .cloned()
            .filter(|c| !c.trim().is_empty());

        let bucket = buckets
            .entry(category_id.clone())
            .or_insert_with(|| CategoryBucket {
                order: category_order,
                id: category_id.clone(),
                name: category_name.clone(),
                icon: category_icon.clone(),
                assets: Vec::new(),
            });

        let asset_index = bucket.assets.len() + 1;
        let editable_fields = EDITABLE_ASSET_FIELDS
            .iter()
            .filter_map(|field| {
                fields
                    .get(*field)
                    .map(|value| ((*field).to_string(), value.clone()))
            })
            .collect();
        bucket.assets.push(Asset {
            id: format!("{category_id}-{asset_index}"),
            record_id: record_id.clone(),
            category_value: category_raw,
            editable_fields,
            name,
            details,
            value,
            status,
            status_label,
            note,
            client_comment: None,
            value_help: None,
            excluded_from_total: category_id == "anticipated" || category_id == "529",
        });
    }

    let mut category_list: Vec<CategoryBucket> = buckets.into_values().collect();
    category_list.sort_by_key(|b| b.order);

    category_list
        .into_iter()
        .map(|bucket| AssetCategory {
            id: bucket.id,
            name: bucket.name,
            icon: bucket.icon,
            assets: bucket.assets,
        })
        .collect()
}

pub fn singular_category_name(name: &str) -> &str {
    match name {
        "Cash Accounts" => "Cash Account",
        "Investment Accounts" => "Investment Account",
        "Retirement Accounts" => "Retirement Account",
        "Annuities" => "Annuity",
        "Life Insurance" => "Life Insurance Policy",
        "529 Plans" => "529 Plan",
        "Personal Effects" => "Personal Effect",
        "Other Assets" => "Other Asset",
        "Bonds" => "Bond",
        "Real Property" => "Real Property",
        other => other,
    }
}

fn is_account_number_str(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }
    // If it contains underscores only, e.g. "____________________", treat as placeholder account
    if trimmed.chars().all(|c| c == '_') {
        return true;
    }
    // If it contains letters (except trailing/leading like x1234), it's likely a description
    let letter_count = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    let digit_count = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    let hyphen_count = trimmed.chars().filter(|c| *c == '-').count();

    if digit_count > 0 && letter_count == 0 {
        return true;
    }
    if digit_count >= 4 && letter_count <= 1 {
        return true;
    }
    if digit_count > 0 && hyphen_count > 0 && letter_count == 0 {
        return true;
    }
    false
}

fn format_masked_acct(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().all(|c| c == '_') {
        return "x____".to_string();
    }
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 4 {
        format!("x{}", &digits[digits.len() - 4..])
    } else if !trimmed.is_empty() {
        format!("x{trimmed}")
    } else {
        String::new()
    }
}

fn field_name_from_collection_value(value: &str) -> String {
    value
        .rsplit("--")
        .next()
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn category_meta_for(name: &str) -> (String, String, String, usize) {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("01") || lower.contains("cash") {
        (
            "cash".to_string(),
            "Cash Accounts".to_string(),
            "$".to_string(),
            1,
        )
    } else if lower.starts_with("02") || lower.contains("investment") {
        (
            "investments".to_string(),
            "Investment Accounts".to_string(),
            "↗".to_string(),
            2,
        )
    } else if lower.starts_with("03") || lower.contains("stock") {
        (
            "stocks".to_string(),
            "Stocks".to_string(),
            "▤".to_string(),
            3,
        )
    } else if lower.starts_with("04") || lower.contains("bond") {
        ("bonds".to_string(), "Bonds".to_string(), "B".to_string(), 4)
    } else if lower.starts_with("05") || lower.contains("personal") || lower.contains("effect") {
        (
            "personal-effects".to_string(),
            "Personal Effects".to_string(),
            "◇".to_string(),
            5,
        )
    } else if lower.starts_with("06") || lower.contains("retirement") {
        (
            "retirement".to_string(),
            "Retirement Accounts".to_string(),
            "▣".to_string(),
            6,
        )
    } else if lower.starts_with("07") || lower.contains("life") || lower.contains("insurance") {
        (
            "life-insurance".to_string(),
            "Life Insurance".to_string(),
            "♡".to_string(),
            7,
        )
    } else if lower.starts_with("08") || lower.contains("pension") {
        (
            "pensions".to_string(),
            "Pensions".to_string(),
            "♙".to_string(),
            8,
        )
    } else if lower.starts_with("09") || lower.contains("annuity") {
        (
            "annuities".to_string(),
            "Annuities".to_string(),
            "A".to_string(),
            9,
        )
    } else if lower.starts_with("10") || lower.contains("money owed") {
        (
            "money-owed".to_string(),
            "Money Owed to Client".to_string(),
            "$".to_string(),
            10,
        )
    } else if lower.starts_with("11")
        || lower.contains("partnership")
        || lower.contains("llc")
        || lower.contains("lpa")
    {
        (
            "llcs".to_string(),
            "LLCs or Partnerships".to_string(),
            "LP".to_string(),
            11,
        )
    } else if lower.starts_with("12") || lower.contains("corporate") {
        (
            "corporate".to_string(),
            "Corporate Interests".to_string(),
            "C".to_string(),
            12,
        )
    } else if lower.starts_with("13") || lower.contains("sole") {
        (
            "sole-proprietorship".to_string(),
            "Sole Proprietorship".to_string(),
            "S".to_string(),
            13,
        )
    } else if lower.starts_with("14")
        || lower.contains("oil")
        || lower.contains("gas")
        || lower.contains("mineral")
    {
        (
            "oil-gas".to_string(),
            "Oil, Gas & Mineral".to_string(),
            "◇".to_string(),
            14,
        )
    } else if lower.starts_with("15") || lower.contains("anticipated") {
        (
            "anticipated".to_string(),
            "Anticipated Asset".to_string(),
            "A".to_string(),
            15,
        )
    } else if lower.starts_with("16")
        || lower.contains("real property")
        || lower.contains("real estate")
    {
        (
            "real-property".to_string(),
            "Real Property".to_string(),
            "⌂".to_string(),
            16,
        )
    } else if lower.starts_with("17") || lower.contains("other") {
        (
            "other".to_string(),
            "Other Assets".to_string(),
            "$".to_string(),
            17,
        )
    } else if lower.starts_with("18") || lower.contains("529") {
        (
            "529".to_string(),
            "529 Plans".to_string(),
            "529".to_string(),
            18,
        )
    } else {
        let slug = lower
            .replace([' ', '-', '/', '&', '(', ')', '.', ','], "_")
            .trim_matches('_')
            .to_string();
        (slug, trimmed.to_string(), "•".to_string(), 99)
    }
}

fn parse_asset_value(raw: &str) -> i64 {
    raw.replace(['$', ',', ' '], "")
        .trim()
        .parse::<f64>()
        .map(|value| value.round() as i64)
        .unwrap_or(0)
}

fn parse_asset_funding_status(raw: &str) -> (AssetStatus, String) {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "verified" => (AssetStatus::Verified, "Verified".to_string()),
        "needs verified" | "needs verification" | "needs to be verified" => (
            AssetStatus::NeedsVerification,
            "Needs to Be Verified".to_string(),
        ),
        "verified title, need ins" => (
            AssetStatus::NeedsVerification,
            "Verified Title, Need Ins".to_string(),
        ),
        "not fundable" => (AssetStatus::NotFundable, "Not Fundable".to_string()),
        "not fund per client" | "not funded per client" | "not funded" => {
            (AssetStatus::NotFunded, "Not Funded Per Client".to_string())
        }
        "undecided" => (AssetStatus::Undecided, "Undecided".to_string()),
        _ => (
            AssetStatus::Undecided,
            if trimmed.is_empty() {
                "Undecided".to_string()
            } else {
                trimmed.to_string()
            },
        ),
    }
}

pub fn refresh_summary(page: &mut AssetsPage) {
    let assets: Vec<&Asset> = page
        .categories
        .iter()
        .flat_map(|category| &category.assets)
        .collect();
    let verified = assets
        .iter()
        .filter(|asset| asset.status == AssetStatus::Verified)
        .count();
    let not_fundable = assets
        .iter()
        .filter(|asset| asset.status == AssetStatus::NotFundable)
        .count();
    let not_funded = assets
        .iter()
        .filter(|asset| asset.status == AssetStatus::NotFunded)
        .count();
    let fundable_total = assets.len().saturating_sub(not_fundable);

    page.funding.verified = verified;
    page.funding.not_fundable = not_fundable;
    page.funding.not_funded_per_client = not_funded;
    page.funding.fundable_total = fundable_total;
    page.funding.left_to_verify = fundable_total.saturating_sub(verified);
    page.funding.percent = if fundable_total == 0 {
        0.0
    } else {
        (verified as f64 * 1000.0 / fundable_total as f64).round() / 10.0
    };
    page.total_value = assets
        .iter()
        .filter(|asset| {
            !asset.excluded_from_total
                && asset.status != AssetStatus::NotFundable
                && asset.status != AssetStatus::NotFunded
        })
        .map(|asset| asset.value)
        .sum();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Form, Path, Query, State},
        http::{HeaderMap, StatusCode},
        routing::{get, post, put},
    };
    use std::collections::HashMap;

    static REFRESH_REQUESTS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    static WRITE_CALLS: std::sync::Mutex<Vec<(String, String, serde_json::Value)>> =
        std::sync::Mutex::new(Vec::new());

    async fn issue_token(
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Json<serde_json::Value> {
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Basic dGVzdC1jbGllbnQ6dGVzdC1zZWNyZXQ=")
        );
        match form.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                assert_eq!(form.get("code").map(String::as_str), Some("test-auth-code"));
                assert_eq!(
                    form.get("redirect_uri").map(String::as_str),
                    Some("http://127.0.0.1:7890/")
                );
                Json(serde_json::json!({
                    "access_token": "initial-access-token",
                    "refresh_token": "test-refresh-token",
                    "expires_in": 0
                }))
            }
            Some("refresh_token") => {
                assert_eq!(
                    form.get("refresh_token").map(String::as_str),
                    Some("test-refresh-token")
                );
                REFRESH_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Json(serde_json::json!({
                    "access_token": "refreshed-access-token",
                    "expires_in": 3600
                }))
            }
            _ => panic!("unexpected OAuth grant"),
        }
    }

    async fn return_assets(
        State(_expected_authorization): State<String>,
        Query(query): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Json<serde_json::Value> {
        let authorization = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(authorization.starts_with("Bearer "));
        let (record_id, description) = if query.get("page").map(String::as_str) == Some("2") {
            ("asset-2", "Checking")
        } else {
            ("asset-1", "Savings")
        };
        let mut response = serde_json::json!({
            "datacollectionrecordvalues": [
                {
                    "id": "category",
                    "stringValue": "01 - Cash Accounts",
                    "links": {
                        "dataCollectionField": "fields--Asset_Category",
                        "dataCollectionRecord": record_id
                    }
                },
                {
                    "id": "description",
                    "stringValue": description,
                    "links": {
                        "dataCollectionField": "fields--Asset_Description",
                        "dataCollectionRecord": record_id
                    }
                }
            ]
        });
        if query.get("page").map(String::as_str) != Some("2") {
            response["meta"] = serde_json::json!({
                "paging": {
                    "datacollectionrecordvalues": {
                        "nextPage": "2"
                    }
                }
            });
        }
        Json(response)
    }

    async fn create_record(
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        assert!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .starts_with("Bearer ")
        );
        assert_eq!(
            body,
            serde_json::json!({
                "datacollectionrecords": {
                    "links": {
                        "action": "7546",
                        "dataCollection": "105"
                    }
                }
            })
        );
        WRITE_CALLS.lock().unwrap().push((
            "POST".to_string(),
            "/api/rest/datacollectionrecords".to_string(),
            body,
        ));
        Json(serde_json::json!({"datacollectionrecords": {"id": 90001}}))
    }

    async fn update_record_values(
        Path(ids): Path<String>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> StatusCode {
        assert!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .starts_with("Bearer ")
        );
        WRITE_CALLS
            .lock()
            .unwrap()
            .push(("PUT".to_string(), ids, body));
        StatusCode::NO_CONTENT
    }

    #[test]
    fn parses_actionstep_records_into_categories() {
        let payload: ActionStepAssetResponse = serde_json::from_value(serde_json::json!({
            "datacollectionrecordvalues": [
                {
                    "id": "category",
                    "stringValue": "01 - Cash Accounts",
                    "links": {
                        "dataCollectionField": "fields--Asset_Category",
                        "dataCollectionRecord": "asset-1"
                    }
                },
                {
                    "id": "description",
                    "stringValue": "Savings",
                    "links": {
                        "dataCollectionField": "fields--Asset_Description",
                        "dataCollectionRecord": "asset-1"
                    }
                },
                {
                    "id": "value",
                    "numericValue": 25000,
                    "links": {
                        "dataCollectionField": "fields--Asset_Value",
                        "dataCollectionRecord": "asset-1"
                    }
                },
                {
                    "id": "funding",
                    "stringValue": "Verified",
                    "links": {
                        "dataCollectionField": "fields--Funding_Status",
                        "dataCollectionRecord": "asset-1"
                    }
                },
                {
                    "id": "deleted",
                    "stringValue": "DELETED",
                    "links": {
                        "dataCollectionField": "fields--Deleted",
                        "dataCollectionRecord": "asset-2"
                    }
                }
            ]
        }))
        .unwrap();
        let categories = build_categories_from_remote_records(&payload.datacollectionrecordvalues);

        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "Cash Accounts");
        assert_eq!(categories[0].assets.len(), 1);
        assert_eq!(categories[0].assets[0].name, "Savings");
        assert_eq!(categories[0].assets[0].value, 25000);
        assert_eq!(categories[0].assets[0].status, AssetStatus::Verified);
        assert_eq!(categories[0].assets[0].record_id, "asset-1");
        assert_eq!(
            categories[0].assets[0].editable_fields["Asset_Description"],
            "Savings"
        );
        assert_eq!(categories[0].assets[0].category_value, "01 - Cash Accounts");
    }

    #[tokio::test]
    async fn signs_in_refreshes_and_fetches_paginated_assets() {
        REFRESH_REQUESTS.store(0, std::sync::atomic::Ordering::SeqCst);
        let app = Router::new().route("/token", post(issue_token)).route(
            "/assets",
            get(return_assets).with_state("Bearer test-access-token".to_string()),
        );
        let app = app
            .route("/api/rest/datacollectionrecords", post(create_record))
            .route(
                "/api/rest/datacollectionrecordvalues/{ids}",
                put(update_record_values),
            );
        WRITE_CALLS.lock().unwrap().clear();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let origin = format!("http://{address}");
        let store = AssetsStore::with_oauth_config(
            format!("{origin}/assets?action=7546&dataCollection=105"),
            format!("{origin}/token"),
            "test-client",
            "test-secret",
        );
        let authorize_url = reqwest::Url::parse(&store.begin_login().await.unwrap()).unwrap();
        let state = authorize_url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        assert!(
            store
                .complete_login("test-auth-code", "wrong-state")
                .await
                .is_err()
        );
        let handoff = store
            .complete_login("test-auth-code", &state)
            .await
            .unwrap();
        let session_id = store.consume_handoff(&handoff).await.unwrap();
        assert!(store.consume_handoff(&handoff).await.is_err());

        let categories = store.get_categories(&session_id).await.unwrap();
        store
            .create_asset(
                &session_id,
                NewAssetInput {
                    category: "01 - Cash Account".to_string(),
                    description: "Checking".to_string(),
                    report_acct_no: "x1234".to_string(),
                    asset_type: "Checking".to_string(),
                    value: "25000".to_string(),
                    comments: "Added from portal".to_string(),
                },
            )
            .await
            .unwrap();
        store
            .update_asset(
                &session_id,
                "2819",
                HashMap::from([
                    ("Asset_Value".to_string(), "12500".to_string()),
                    ("LienAmount".to_string(), "1000".to_string()),
                ]),
            )
            .await
            .unwrap();
        store.delete_asset(&session_id, "2819").await.unwrap();
        server.abort();

        assert_eq!(
            REFRESH_REQUESTS.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].assets.len(), 2);
        assert_eq!(categories[0].assets[0].name, "Savings");
        assert_eq!(categories[0].assets[1].name, "Checking");
        let writes = WRITE_CALLS.lock().unwrap();
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[0].0, "POST");
        let create_values = writes[1]
            .2
            .get("datacollectionrecordvalues")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert!(
            create_values
                .iter()
                .any(|value| value["id"] == "105--Client_Added--90001"
                    && value["stringValue"] == "T")
        );
        let updated_values = writes[2]
            .2
            .get("datacollectionrecordvalues")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(updated_values.len(), 2);
        assert!(
            updated_values
                .iter()
                .all(|value| value["id"] != "105--Client_Added--2819")
        );
        let deleted_values = writes[3]
            .2
            .get("datacollectionrecordvalues")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(deleted_values[0]["id"], "105--Deleted--2819");
        assert_eq!(deleted_values[0]["stringValue"], "DELETED");
    }
}
