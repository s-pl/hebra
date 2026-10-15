//! Signing in with GitHub (OAuth web flow). The web app sends the user to
//! GitHub and back with a code; the server exchanges it, using a secret only
//! it knows, for the GitHub identity.

use serde::Deserialize;

use crate::config::GithubOauth;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone)]
pub struct GithubUser {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct Email {
    email: String,
    primary: bool,
    verified: bool,
}

const USER_AGENT: &str = concat!("hebra-server/", env!("CARGO_PKG_VERSION"));

/// Exchanges an OAuth code for the GitHub user who approved it.
pub async fn exchange(
    http: &reqwest::Client,
    config: &GithubOauth,
    code: &str,
    redirect_uri: &str,
) -> ApiResult<GithubUser> {
    let token: TokenResponse = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| ApiError::internal("github", e))?
        .json()
        .await
        .map_err(|e| ApiError::internal("github", e))?;
    let Some(access) = token.access_token else {
        return Err(ApiError::unauthorized(
            token
                .error_description
                .as_deref()
                .unwrap_or("GitHub did not accept the sign-in; try again"),
        ));
    };
    let get = |url: &'static str| {
        http.get(url)
            .bearer_auth(&access)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", USER_AGENT)
            .send()
    };
    let user: User = get("https://api.github.com/user")
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| ApiError::internal("github", e))?
        .json()
        .await
        .map_err(|e| ApiError::internal("github", e))?;
    // Only a primary, verified address identifies a person.
    let emails: Vec<Email> = match get("https://api.github.com/user/emails").await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        _ => Vec::new(),
    };
    let email = emails
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email);
    Ok(GithubUser {
        id: user.id.to_string(),
        login: user.login,
        name: user.name,
        email,
        avatar_url: user.avatar_url,
    })
}
