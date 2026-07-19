//! Minimal GitHub GraphQL client (spec 00 §4.1): stargazers pagination with
//! `starredAt` timestamps. GraphQL-only because the REST stargazers endpoint
//! was restricted to repo admins/collaborators on 2026-07-14, while the
//! GraphQL connection remains readable by any authenticated identity
//! (including `GITHUB_TOKEN` and App installation tokens).

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;

use crate::{Result, ScopeError};

const API_BASE: &str = "https://api.github.com";
const MAX_RETRIES: u32 = 5;
/// Cap on rate-limit sleeps per attempt so CI stays responsive.
const MAX_RATE_SLEEP_SECS: u64 = 600;

const STARGAZERS_QUERY: &str = r#"query($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    stargazers(first: 100, after: $after) {
      totalCount
      pageInfo { hasNextPage endCursor }
      edges { starredAt }
    }
  }
  rateLimit { remaining resetAt }
}"#;

pub struct GitHubClient {
    http: reqwest::Client,
    base: String,
    graphql_url: String,
    /// Injectable backoff base (ms); tests set it to ~0.
    backoff_base_ms: u64,
}

/// A repo contributor from the REST contributors endpoint.
#[derive(Debug, Deserialize)]
pub struct Contributor {
    pub login: Option<String>,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub contributions: u64,
    #[serde(rename = "type", default)]
    pub kind: String,
}

impl Contributor {
    pub fn is_bot(&self) -> bool {
        self.kind == "Bot"
    }
}

/// One page of the stargazers connection.
#[derive(Debug)]
pub struct StarPage {
    pub total_count: u64,
    /// `starredAt` RFC 3339 timestamps in this page (chronological).
    pub starred_at: Vec<String>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
    /// Rate-limit points remaining (when the API reports it).
    pub rate_remaining: Option<u64>,
}

#[derive(Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize)]
struct GqlData {
    repository: Option<GqlRepo>,
    #[serde(rename = "rateLimit")]
    rate_limit: Option<GqlRateLimit>,
}

#[derive(Deserialize)]
struct GqlRepo {
    stargazers: GqlStars,
}

#[derive(Deserialize)]
struct GqlStars {
    #[serde(rename = "totalCount")]
    total_count: u64,
    #[serde(rename = "pageInfo")]
    page_info: GqlPageInfo,
    edges: Vec<GqlEdge>,
}

#[derive(Deserialize)]
struct GqlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct GqlEdge {
    #[serde(rename = "starredAt")]
    starred_at: String,
}

#[derive(Deserialize)]
struct GqlError {
    message: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct GqlRateLimit {
    remaining: u64,
    #[serde(rename = "resetAt")]
    reset_at: String,
}

impl GitHubClient {
    pub fn new(token: &str) -> Result<Self> {
        Self::with_base(API_BASE.to_string(), token, 1000)
    }

    pub fn with_base(base: String, token: &str, backoff_base_ms: u64) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("reposcope/{}", env!("CARGO_PKG_VERSION"))).unwrap(),
        );
        let v = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| ScopeError::Usage("token contains invalid characters".into()))?;
        headers.insert(AUTHORIZATION, v);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ScopeError::Api(format!("http client build: {e}")))?;
        Ok(Self {
            http,
            graphql_url: format!("{base}/graphql"),
            base,
            backoff_base_ms,
        })
    }

    /// One page of repo contributors (REST, 100 per page).
    pub async fn contributors_page(&self, repo: &str, page: u32) -> Result<Vec<Contributor>> {
        let path = format!("/repos/{repo}/contributors?per_page=100&page={page}");
        let (body, _) = self.get_with_retry(&path).await?;
        serde_json::from_str(&body).map_err(|e| ScopeError::Api(format!("GET {path}: decode: {e}")))
    }

    /// Download raw bytes (e.g. an avatar image) from an absolute URL.
    /// Returns `(content-type, bytes)`.
    pub async fn fetch_bytes(&self, url: &str) -> Result<(String, Vec<u8>)> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| ScopeError::Api(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(ScopeError::Api(format!(
                "GET {url}: HTTP {}",
                resp.status()
            )));
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
            .filter(|s| s.starts_with("image/"))
            .unwrap_or_else(|| "image/png".to_string());
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ScopeError::Api(format!("GET {url}: read: {e}")))?;
        Ok((mime, bytes.to_vec()))
    }

    /// GET a REST API path with retries; returns `(body, headers)`.
    async fn get_with_retry(&self, path: &str) -> Result<(String, reqwest::header::HeaderMap)> {
        let url = format!("{}{}", self.base, path);
        let mut attempt = 0u32;
        loop {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| ScopeError::Api(format!("GET {path}: {e}")))?;
            let status = resp.status();
            if status.is_success() {
                let headers = resp.headers().clone();
                let body = resp
                    .text()
                    .await
                    .map_err(|e| ScopeError::Api(format!("GET {path}: read body: {e}")))?;
                return Ok((body, headers));
            }
            if status.as_u16() == 404 {
                return Err(ScopeError::Api(
                    "repository not found (or token lacks access)".into(),
                ));
            }
            attempt += 1;
            let retriable =
                status.as_u16() == 403 || status.as_u16() == 429 || status.is_server_error();
            if !retriable || attempt > MAX_RETRIES {
                let body = resp.text().await.unwrap_or_default();
                return Err(ScopeError::Api(format!(
                    "GET {path}: HTTP {status} after {attempt} attempt(s): {}",
                    &body[..body.len().min(200)]
                )));
            }
            let hinted = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| std::time::Duration::from_secs(s.min(MAX_RATE_SLEEP_SECS)));
            let backoff =
                std::time::Duration::from_millis(self.backoff_base_ms * 2u64.pow(attempt - 1));
            tokio::time::sleep(hinted.unwrap_or(backoff)).await;
        }
    }

    /// Fetch one stargazers page (100 edges) after the given cursor.
    pub async fn stargazers_page(&self, repo: &str, after: Option<&str>) -> Result<StarPage> {
        let (owner, name) = repo
            .split_once('/')
            .ok_or_else(|| ScopeError::Usage(format!("invalid repo {repo:?}")))?;
        let body = serde_json::json!({
            "query": STARGAZERS_QUERY,
            "variables": { "owner": owner, "name": name, "after": after },
        });
        let mut attempt = 0u32;
        loop {
            let resp = self
                .http
                .post(&self.graphql_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| ScopeError::Api(format!("POST /graphql: {e}")))?;
            let status = resp.status();
            attempt += 1;
            if status.is_server_error() || status.as_u16() == 429 {
                if attempt > MAX_RETRIES {
                    return Err(ScopeError::Api(format!(
                        "POST /graphql: HTTP {status} after {MAX_RETRIES} retries"
                    )));
                }
                let hinted = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|s| std::time::Duration::from_secs(s.min(MAX_RATE_SLEEP_SECS)));
                let backoff =
                    std::time::Duration::from_millis(self.backoff_base_ms * 2u64.pow(attempt - 1));
                tokio::time::sleep(hinted.unwrap_or(backoff)).await;
                continue;
            }
            let text = resp
                .text()
                .await
                .map_err(|e| ScopeError::Api(format!("POST /graphql: read body: {e}")))?;
            if !status.is_success() {
                return Err(ScopeError::Api(format!(
                    "POST /graphql: HTTP {status}: {}",
                    &text[..text.len().min(200)]
                )));
            }
            let gql: GqlResponse = serde_json::from_str(&text)
                .map_err(|e| ScopeError::Api(format!("POST /graphql: decode: {e}")))?;
            if let Some(errors) = gql.errors
                && !errors.is_empty()
            {
                if errors
                    .iter()
                    .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
                    && attempt <= MAX_RETRIES
                {
                    let reset = gql
                        .data
                        .as_ref()
                        .and_then(|d| d.rate_limit.as_ref())
                        .and_then(|r| chrono::DateTime::parse_from_rfc3339(&r.reset_at).ok())
                        .map(|t| {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            (t.timestamp().max(0) as u64).saturating_sub(now).max(1)
                        })
                        .unwrap_or(30);
                    tokio::time::sleep(std::time::Duration::from_secs(
                        reset.min(MAX_RATE_SLEEP_SECS),
                    ))
                    .await;
                    continue;
                }
                return Err(ScopeError::Api(errors[0].message.clone()));
            }
            let data = gql
                .data
                .ok_or_else(|| ScopeError::Api("POST /graphql: empty data".into()))?;
            let repo = data.repository.ok_or_else(|| {
                ScopeError::Api("repository not found (or token lacks access)".into())
            })?;
            let sg = repo.stargazers;
            return Ok(StarPage {
                total_count: sg.total_count,
                starred_at: sg.edges.into_iter().map(|e| e.starred_at).collect(),
                has_next_page: sg.page_info.has_next_page,
                end_cursor: sg.page_info.end_cursor,
                rate_remaining: data.rate_limit.map(|r| r.remaining),
            });
        }
    }
}
