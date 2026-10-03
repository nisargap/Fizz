use std::{env, time::Duration};

use reqwest::{Client, StatusCode, Url, header};
use serde_json::Value;

/// Supabase is accessed only from server-side Rust functions. Never send this
/// client or its service role key to the browser.
pub struct Supabase {
    client: Client,
    rest_url: Url,
    service_role_key: String,
}

#[derive(Debug)]
pub enum ConnectionError {
    MissingConfiguration,
    InvalidUrl,
    RequestFailed,
    Unauthorized,
    UnexpectedStatus,
}

#[derive(Debug)]
pub enum StoreError {
    Conflict,
    Unavailable,
}

impl Supabase {
    pub fn from_env() -> Result<Self, ConnectionError> {
        let project_url =
            env::var("SUPABASE_URL").map_err(|_| ConnectionError::MissingConfiguration)?;
        let service_role_key = env::var("SUPABASE_SERVICE_ROLE_KEY")
            .map_err(|_| ConnectionError::MissingConfiguration)?;
        if service_role_key.trim().is_empty() {
            return Err(ConnectionError::MissingConfiguration);
        }

        let mut rest_url = Url::parse(&project_url).map_err(|_| ConnectionError::InvalidUrl)?;
        if rest_url.scheme() != "https"
            && !(rest_url.scheme() == "http"
                && matches!(rest_url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
        {
            return Err(ConnectionError::InvalidUrl);
        }
        if rest_url.host_str().is_none()
            || rest_url.username() != ""
            || rest_url.password().is_some()
        {
            return Err(ConnectionError::InvalidUrl);
        }
        rest_url.set_path("/rest/v1/");
        rest_url.set_query(None);
        rest_url.set_fragment(None);

        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| ConnectionError::RequestFailed)?;
        Ok(Self {
            client,
            rest_url,
            service_role_key,
        })
    }

    /// Verify that PostgREST accepts the configured service role credential.
    pub async fn check_connection(&self) -> Result<(), ConnectionError> {
        let response = self
            .client
            .get(self.rest_url.clone())
            .header("apikey", &self.service_role_key)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.service_role_key),
            )
            .send()
            .await
            .map_err(|_| ConnectionError::RequestFailed)?;
        match response.status() {
            StatusCode::OK => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ConnectionError::Unauthorized),
            _ => Err(ConnectionError::UnexpectedStatus),
        }
    }

    pub async fn rpc(&self, name: &str, payload: Value) -> Result<Value, StoreError> {
        let url = self
            .rest_url
            .join(&format!("rpc/{name}"))
            .map_err(|_| StoreError::Unavailable)?;
        let response = self
            .client
            .post(url)
            .header("apikey", &self.service_role_key)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.service_role_key),
            )
            .json(&payload)
            .send()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if response.status().is_success() {
            if response.status() == StatusCode::NO_CONTENT {
                return Ok(Value::Null);
            }
            return response.json().await.map_err(|_| StoreError::Unavailable);
        }
        if response.status() == StatusCode::CONFLICT {
            return Err(StoreError::Conflict);
        }
        Err(StoreError::Unavailable)
    }
}
