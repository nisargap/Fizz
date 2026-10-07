use std::{env, time::Duration};

use reqwest::{Client, Method, StatusCode, Url, header};
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

    /// Where the browser starts a Supabase Auth OAuth sign-in. Supabase sends it back to
    /// `redirect_to` with a single-use `code` that only the holder of the PKCE verifier can use.
    pub fn authorize_url(&self, provider: &str, redirect_to: &str, code_challenge: &str) -> Url {
        let mut url = self.rest_url.clone();
        url.set_path("/auth/v1/authorize");
        url.query_pairs_mut()
            .append_pair("provider", provider)
            .append_pair("redirect_to", redirect_to)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "s256");
        url
    }

    /// Call a Supabase Auth endpoint, such as `token` or `passkeys/authentication/options`, and
    /// return its status and JSON body so callers can map Supabase's error codes. `bearer` is a
    /// user's access token for endpoints that act as that user. `client_ip` is forwarded so
    /// Supabase's per-IP rate limits see the person rather than this server.
    pub async fn auth(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
        body: Option<Value>,
        client_ip: Option<&str>,
    ) -> Result<(StatusCode, Value), StoreError> {
        let mut url = self.rest_url.clone();
        url.set_path(&format!("/auth/v1/{path}"));
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut request = self
            .client
            .request(method, url)
            .header("apikey", &self.service_role_key)
            // Errors come back as {"code": "<error_code>", "message": ...}.
            .header("x-supabase-api-version", "2024-01-01");
        if let Some(token) = bearer {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(ip) = client_ip {
            request = request.header("x-forwarded-for", ip);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| StoreError::Unavailable)?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_targets_supabase_auth_with_an_encoded_redirect() {
        let store = Supabase {
            client: Client::new(),
            rest_url: Url::parse("https://ref.supabase.co/rest/v1/").unwrap(),
            service_role_key: "key".into(),
        };
        assert_eq!(
            store
                .authorize_url("google", "https://fizzlayer.com/api/auth", "abc")
                .as_str(),
            "https://ref.supabase.co/auth/v1/authorize?provider=google&redirect_to=https%3A%2F%2Ffizzlayer.com%2Fapi%2Fauth&code_challenge=abc&code_challenge_method=s256"
        );
    }
}
