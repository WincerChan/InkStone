use chrono::{DateTime, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use thiserror::Error;

const GRAPHQL_ENDPOINT: &str = "https://api.github.com/graphql";
const REST_ENDPOINT: &str = "https://api.github.com";
const USER_AGENT: &str = "inkstone";

#[derive(Debug, Error)]
pub enum GithubError {
    #[error("jwt error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("graphql error: {0}")]
    Graphql(String),
    #[error("missing data: {0}")]
    MissingData(&'static str),
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),
}

#[derive(Debug, Clone)]
pub struct GithubAppClient {
    http: reqwest::Client,
    app_id: u64,
    installation_id: u64,
    private_key: String,
}

#[derive(Debug, Clone)]
pub struct DiscussionInfo {
    pub id: String,
    pub number: i32,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub comments: Vec<DiscussionComment>,
}

#[derive(Debug, Clone)]
pub struct DiscussionComment {
    pub id: String,
    pub author_login: Option<String>,
    pub author_url: Option<String>,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub replies: Vec<DiscussionComment>,
}

impl GithubAppClient {
    pub fn new(
        http: reqwest::Client,
        app_id: u64,
        installation_id: u64,
        private_key: String,
    ) -> Self {
        Self {
            http,
            app_id,
            installation_id,
            private_key,
        }
    }

    pub async fn find_discussion_by_title(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
    ) -> Result<Option<DiscussionInfo>, GithubError> {
        let query = r#"
            query($query: String!) {
              search(type: DISCUSSION, query: $query, first: 10) {
                nodes {
                  ... on Discussion {
                    id
                    number
                    title
                    url
                    createdAt
                    updatedAt
                  }
                }
              }
            }
        "#;
        let search_query = format!("repo:{owner}/{repo} in:title {title}");
        let data: SearchResponse = self.graphql(query, SearchVars { query: search_query }).await?;
        let nodes = data.search.nodes.unwrap_or_default();
        for node in nodes {
            if node.title == title {
                return Ok(Some(node.into_info(Vec::new())?));
            }
        }
        Ok(None)
    }

    pub async fn fetch_discussion_by_id(
        &self,
        discussion_id: &str,
    ) -> Result<DiscussionInfo, GithubError> {
        let query = r#"
            query($id: ID!) {
              node(id: $id) {
                ... on Discussion {
                  id
                  number
                  title
                  url
                  createdAt
                  updatedAt
                  comments(first: 100) {
                    nodes {
                      id
                      bodyHTML
                      createdAt
                      updatedAt
                      author { login url }
                      replies(first: 100) {
                        nodes {
                          id
                          bodyHTML
                          createdAt
                          updatedAt
                          author { login url }
                        }
                      }
                    }
                  }
                }
              }
            }
        "#;
        let data: DiscussionNodeResponse = self.graphql(query, NodeVars { id: discussion_id }).await?;
        let node = data.node.ok_or(GithubError::MissingData("discussion node"))?;
        node.into_info()
    }

    pub async fn create_discussion(
        &self,
        owner: &str,
        repo: &str,
        category_id: &str,
        title: &str,
        body: &str,
    ) -> Result<DiscussionInfo, GithubError> {
        let repo_id = self.fetch_repository_id(owner, repo).await?;
        let query = r#"
            mutation($repo: ID!, $category: ID!, $title: String!, $body: String!) {
              createDiscussion(input: { repositoryId: $repo, categoryId: $category, title: $title, body: $body }) {
                discussion {
                  id
                  number
                  title
                  url
                  createdAt
                  updatedAt
                }
              }
            }
        "#;
        let vars = CreateDiscussionVars {
            repo: repo_id,
            category: category_id.to_string(),
            title: title.to_string(),
            body: body.to_string(),
        };
        let data: CreateDiscussionResponse = self.graphql(query, vars).await?;
        let discussion = data
            .create_discussion
            .and_then(|payload| payload.discussion)
            .ok_or(GithubError::MissingData("createDiscussion"))?;
        discussion.into_info(Vec::new())
    }

    async fn fetch_repository_id(&self, owner: &str, repo: &str) -> Result<String, GithubError> {
        let query = r#"
            query($owner: String!, $name: String!) {
              repository(owner: $owner, name: $name) { id }
            }
        "#;
        let data: RepositoryResponse = self
            .graphql(
                query,
                RepositoryVars {
                    owner: owner.to_string(),
                    name: repo.to_string(),
                },
            )
            .await?;
        let repo = data.repository.ok_or(GithubError::MissingData("repository"))?;
        Ok(repo.id)
    }

    async fn graphql<T, V>(&self, query: &str, variables: V) -> Result<T, GithubError>
    where
        T: for<'de> Deserialize<'de>,
        V: Serialize,
    {
        let token = self.installation_token().await?;
        let response = self
            .http
            .post(GRAPHQL_ENDPOINT)
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .json(&GraphqlRequest { query, variables })
            .send()
            .await?
            .error_for_status()?;
        let payload: GraphqlResponse<T> = response.json().await?;
        if let Some(errors) = payload.errors {
            let message = errors
                .into_iter()
                .map(|err| err.message)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(GithubError::Graphql(message));
        }
        payload.data.ok_or(GithubError::MissingData("graphql data"))
    }

    async fn installation_token(&self) -> Result<String, GithubError> {
        let jwt = self.app_jwt()?;
        let url = format!("{REST_ENDPOINT}/app/installations/{}/access_tokens", self.installation_id);
        let response = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(GithubError::InvalidResponse(format!(
                "installation token request failed: status {status}, body {body}"
            )));
        }
        let payload: InstallationTokenResponse = from_str(&body)
            .map_err(|_| GithubError::InvalidResponse(format!("invalid token payload: {body}")))?;
        if payload.token.trim().is_empty() {
            return Err(GithubError::InvalidResponse("missing token".to_string()));
        }
        Ok(payload.token)
    }

    fn app_jwt(&self) -> Result<String, GithubError> {
        let now = Utc::now().timestamp();
        let claims = AppClaims {
            iat: now - 60,
            exp: now + 300,
            iss: self.app_id,
        };
        let key = EncodingKey::from_rsa_pem(self.private_key.as_bytes())?;
        Ok(encode(&Header::new(jsonwebtoken::Algorithm::RS256), &claims, &key)?)
    }
}

#[derive(Debug, Serialize)]
struct GraphqlRequest<'a, V> {
    query: &'a str,
    variables: V,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlErrorItem>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlErrorItem {
    message: String,
}

#[derive(Debug, Serialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: u64,
}

#[derive(Debug, Deserialize)]
struct InstallationTokenResponse {
    token: String,
}

#[derive(Debug, Serialize)]
struct RepositoryVars {
    owner: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryResponse {
    repository: Option<RepositoryNode>,
}

#[derive(Debug, Deserialize)]
struct RepositoryNode {
    id: String,
}

#[derive(Debug, Serialize)]
struct SearchVars {
    query: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    search: SearchResult,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    nodes: Option<Vec<DiscussionNode>>,
}

#[derive(Debug, Serialize)]
struct NodeVars<'a> {
    id: &'a str,
}

#[derive(Debug, Deserialize)]
struct DiscussionNodeResponse {
    node: Option<DiscussionNodeWithComments>,
}

#[derive(Debug, Deserialize)]
struct DiscussionNodeWithComments {
    #[serde(flatten)]
    discussion: DiscussionNode,
    comments: CommentConnection,
}

#[derive(Debug, Deserialize)]
struct DiscussionNode {
    id: String,
    number: i32,
    title: String,
    url: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CommentConnection {
    nodes: Vec<CommentNode>,
}

#[derive(Debug, Deserialize)]
struct CommentNode {
    id: String,
    #[serde(rename = "bodyHTML")]
    body_html: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    author: Option<AuthorNode>,
    replies: ReplyConnection,
}

#[derive(Debug, Deserialize)]
struct ReplyConnection {
    nodes: Vec<ReplyNode>,
}

#[derive(Debug, Deserialize)]
struct ReplyNode {
    id: String,
    #[serde(rename = "bodyHTML")]
    body_html: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    author: Option<AuthorNode>,
}

#[derive(Debug, Deserialize)]
struct AuthorNode {
    login: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateDiscussionVars {
    repo: String,
    category: String,
    title: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CreateDiscussionResponse {
    #[serde(rename = "createDiscussion")]
    create_discussion: Option<CreateDiscussionPayload>,
}

#[derive(Debug, Deserialize)]
struct CreateDiscussionPayload {
    discussion: Option<DiscussionNode>,
}

impl DiscussionNode {
    fn into_info(self, comments: Vec<CommentNode>) -> Result<DiscussionInfo, GithubError> {
        let created_at = parse_datetime(&self.created_at)?;
        let updated_at = parse_datetime(&self.updated_at)?;
        let mut mapped_comments = Vec::with_capacity(comments.len());
        for comment in comments {
            mapped_comments.push(map_comment(comment)?);
        }
        Ok(DiscussionInfo {
            id: self.id,
            number: self.number,
            title: self.title,
            url: self.url,
            created_at,
            updated_at,
            comments: mapped_comments,
        })
    }
}

impl DiscussionNodeWithComments {
    fn into_info(self) -> Result<DiscussionInfo, GithubError> {
        self.discussion.into_info(self.comments.nodes)
    }
}

fn map_comment(node: CommentNode) -> Result<DiscussionComment, GithubError> {
    let created_at = parse_datetime(&node.created_at)?;
    let updated_at = parse_datetime(&node.updated_at)?;
    let mut replies = Vec::with_capacity(node.replies.nodes.len());
    for reply in node.replies.nodes {
        replies.push(map_reply(reply)?);
    }
    Ok(DiscussionComment {
        id: node.id,
        author_login: node.author.as_ref().and_then(|author| author.login.clone()),
        author_url: node.author.as_ref().and_then(|author| author.url.clone()),
        body_html: node.body_html,
        created_at,
        updated_at,
        replies,
    })
}

fn map_reply(node: ReplyNode) -> Result<DiscussionComment, GithubError> {
    let created_at = parse_datetime(&node.created_at)?;
    let updated_at = parse_datetime(&node.updated_at)?;
    Ok(DiscussionComment {
        id: node.id,
        author_login: node.author.as_ref().and_then(|author| author.login.clone()),
        author_url: node.author.as_ref().and_then(|author| author.url.clone()),
        body_html: node.body_html,
        created_at,
        updated_at,
        replies: Vec::new(),
    })
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, GithubError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| GithubError::InvalidTimestamp(value.to_string()))
}
