//! Thin GitHub GraphQL client.
//!
//! The two skills this app replaces (`my-prs`, `prs-to-review`) shell out to `gh`
//! once per PR, which is fine for a one-off report but not for a 60s poller —
//! 30 labeled PRs meant 30 subprocesses and 30 round trips. The same fields are
//! available from a single GraphQL search, so each poll is 3 requests total.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const USER_AGENT: &str = "pr-radar/0.1";

pub struct Github {
    token: String,
    client: reqwest::Client,
}

impl Github {
    /// Resolve a token the same way `gh` itself does: explicit env first, then
    /// whatever the logged-in `gh` keyring holds.
    pub async fn discover() -> Result<Self> {
        let token = match std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => gh_auth_token().await?,
        };

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self { token, client })
    }

    async fn graphql<T: DeserializeOwned>(&self, query: &str, vars: serde_json::Value) -> Result<T> {
        let resp = self
            .client
            .post(GRAPHQL_URL)
            .bearer_auth(&self.token)
            .json(&json!({ "query": query, "variables": vars }))
            .send()
            .await
            .context("GitHub GraphQL request failed")?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .context("GitHub returned a non-JSON response")?;

        if !status.is_success() {
            return Err(anyhow!("GitHub returned {}: {}", status, body));
        }

        // GraphQL reports partial failures with HTTP 200 and an `errors` array.
        if let Some(errors) = body.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let msgs: Vec<String> = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .map(|s| s.to_string())
                    .collect();
                return Err(anyhow!("GitHub GraphQL error: {}", msgs.join("; ")));
            }
        }

        let data = body
            .get("data")
            .ok_or_else(|| anyhow!("GitHub response had no data"))?;
        Ok(serde_json::from_value(data.clone())?)
    }

    /// Who am I, and which teams of `org` am I on? Team review requests arrive as
    /// `org/team-slug`, individual ones as a bare login, so we need both to tell
    /// whether a review was actually asked of this user.
    pub async fn viewer(&self, org: &str) -> Result<Viewer> {
        #[derive(Deserialize)]
        struct Resp {
            viewer: ViewerNode,
            organization: Option<OrgNode>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ViewerNode {
            login: String,
            avatar_url: String,
        }
        #[derive(Deserialize)]
        struct OrgNode {
            teams: TeamConn,
        }
        #[derive(Deserialize)]
        struct TeamConn {
            nodes: Vec<TeamNode>,
        }
        #[derive(Deserialize)]
        struct TeamNode {
            slug: String,
        }

        let query = r#"
            query($org: String!, $login: String!) {
              viewer { login avatarUrl }
              organization(login: $org) {
                teams(first: 50, userLogins: [$login]) { nodes { slug } }
              }
            }
        "#;

        // The team lookup needs a login, so resolve the viewer first.
        let me: serde_json::Value = self
            .graphql(r#"query { viewer { login } }"#, json!({}))
            .await?;
        let login = me["viewer"]["login"]
            .as_str()
            .ok_or_else(|| anyhow!("could not resolve viewer login"))?
            .to_string();

        let resp: Resp = self
            .graphql(query, json!({ "org": org, "login": login }))
            .await?;

        let teams = resp
            .organization
            .map(|o| {
                o.teams
                    .nodes
                    .into_iter()
                    .map(|t| format!("{org}/{}", t.slug))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Viewer {
            login: resp.viewer.login,
            avatar_url: resp.viewer.avatar_url,
            teams,
        })
    }

    /// Run a GitHub search and return every node that parses as a pull request.
    pub async fn search_prs(&self, q: &str, limit: u32) -> Result<Vec<RawPr>> {
        #[derive(Deserialize)]
        struct Resp {
            search: SearchConn,
        }
        #[derive(Deserialize)]
        struct SearchConn {
            nodes: Vec<serde_json::Value>,
        }

        let resp: Resp = self
            .graphql(PR_SEARCH_QUERY, json!({ "q": q, "limit": limit }))
            .await?;

        // Search returns a union; non-PR nodes come back as `{}`. Skip anything
        // that does not parse rather than failing the whole poll.
        Ok(resp
            .search
            .nodes
            .into_iter()
            .filter_map(|n| serde_json::from_value::<RawPr>(n).ok())
            .filter(|p| p.number > 0)
            .collect())
    }

    /// Recently merged/closed PRs by the viewer — context only, no status needed.
    pub async fn search_closed(&self, q: &str, limit: u32) -> Result<Vec<ClosedPr>> {
        #[derive(Deserialize)]
        struct Resp {
            search: SearchConn,
        }
        #[derive(Deserialize)]
        struct SearchConn {
            nodes: Vec<serde_json::Value>,
        }

        let query = r#"
            query($q: String!, $limit: Int!) {
              search(query: $q, type: ISSUE, first: $limit) {
                nodes {
                  ... on PullRequest {
                    number title url state closedAt mergedAt
                    repository { nameWithOwner }
                  }
                }
              }
            }
        "#;

        let resp: Resp = self.graphql(query, json!({ "q": q, "limit": limit })).await?;
        Ok(resp
            .search
            .nodes
            .into_iter()
            .filter_map(|n| serde_json::from_value::<ClosedPr>(n).ok())
            .filter(|p| p.number > 0)
            .collect())
    }
}

/// Every place `gh` realistically lives, PATH first and then the known install
/// prefixes.
///
/// This cannot rely on PATH alone. A macOS app launched from Finder inherits a
/// minimal `/usr/bin:/bin:/usr/sbin:/sbin` rather than the login shell's PATH,
/// so a Homebrew `gh` that resolves fine in a terminal is invisible here.
fn gh_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|dir| dir.join("gh")));
    }

    out.extend(
        [
            "/opt/homebrew/bin/gh", // Homebrew, Apple silicon
            "/usr/local/bin/gh",    // Homebrew, Intel + official installer
            "/opt/local/bin/gh",    // MacPorts
            "/usr/bin/gh",          // distro packages
            "/snap/bin/gh",         // Snap
            "/var/lib/snapd/snap/bin/gh",
            "/run/current-system/sw/bin/gh", // NixOS
            "/usr/local/share/gh/bin/gh",
        ]
        .iter()
        .map(PathBuf::from),
    );

    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        out.push(home.join(".local/bin/gh"));
        out.push(home.join("bin/gh"));
        out.push(home.join(".nix-profile/bin/gh"));
    }

    out
}

async fn run_gh_auth_token(program: &Path) -> Result<String> {
    let out = tokio::process::Command::new(program)
        .args(["auth", "token"])
        .output()
        .await
        .with_context(|| format!("could not run {}", program.display()))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("`gh auth token` failed: {}", err.trim()));
    }

    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("`gh auth token` returned nothing"));
    }
    Ok(token)
}

/// Last resort: ask the user's login shell, which sources their profile and so
/// knows the PATH they actually use.
async fn gh_token_via_login_shell() -> Result<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let out = tokio::process::Command::new(&shell)
        .args(["-lc", "gh auth token"])
        .output()
        .await
        .with_context(|| format!("could not run {shell}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("{}", err.trim()));
    }

    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("login shell returned no token"));
    }
    Ok(token)
}

async fn gh_auth_token() -> Result<String> {
    let mut found_any = false;
    let mut last_error: Option<String> = None;

    for candidate in gh_candidates() {
        if !candidate.is_file() {
            continue;
        }
        found_any = true;
        match run_gh_auth_token(&candidate).await {
            Ok(token) => return Ok(token),
            Err(e) => last_error = Some(format!("{e:#}")),
        }
    }

    // `gh` exists somewhere but every copy refused: an auth problem, not a
    // lookup problem. Say so precisely instead of blaming the install.
    if found_any {
        return Err(anyhow!(
            "{} — run `gh auth login`, or set GH_TOKEN",
            last_error.unwrap_or_else(|| "`gh auth token` failed".into())
        ));
    }

    if let Ok(token) = gh_token_via_login_shell().await {
        return Ok(token);
    }

    Err(anyhow!(
        "could not find the `gh` executable. An app launched from the desktop \
         inherits a minimal PATH rather than your shell's, so an install in \
         /opt/homebrew, /snap or ~/.local can be invisible to it. Install the \
         GitHub CLI, or set GH_TOKEN for this app."
    ))
}

const PR_SEARCH_QUERY: &str = r#"
query($q: String!, $limit: Int!) {
  search(query: $q, type: ISSUE, first: $limit) {
    nodes {
      ... on PullRequest {
        number
        title
        url
        createdAt
        updatedAt
        isDraft
        mergeable
        reviewDecision
        author { login }
        repository { nameWithOwner }
        labels(first: 15) { nodes { name } }
        reviewRequests(first: 25) {
          nodes {
            requestedReviewer {
              ... on User { login }
              ... on Team { slug }
            }
          }
        }
        reviews(first: 60) {
          nodes { state submittedAt author { login } }
        }
        commits(last: 1) {
          nodes {
            commit {
              oid
              statusCheckRollup {
                state
                contexts(first: 40) {
                  nodes {
                    ... on CheckRun { name status conclusion startedAt completedAt }
                    ... on StatusContext { context state createdAt }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
"#;

// ---------------------------------------------------------------------------
// Raw response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Viewer {
    pub login: String,
    pub avatar_url: String,
    /// Fully qualified as `org/slug`, matching how team review requests appear.
    pub teams: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub mergeable: Option<String>,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub author: Option<Actor>,
    pub repository: Repo,
    #[serde(default)]
    pub labels: Option<NodeList<Label>>,
    #[serde(default)]
    pub review_requests: Option<NodeList<ReviewRequest>>,
    #[serde(default)]
    pub reviews: Option<NodeList<RawReview>>,
    #[serde(default)]
    pub commits: Option<NodeList<CommitNode>>,
}

impl RawPr {
    pub fn slug(&self) -> String {
        let short = self
            .repository
            .name_with_owner
            .split('/')
            .next_back()
            .unwrap_or(&self.repository.name_with_owner);
        format!("{short}#{}", self.number)
    }

    pub fn id(&self) -> String {
        format!("{}#{}", self.repository.name_with_owner, self.number)
    }

    pub fn author_login(&self) -> &str {
        self.author.as_ref().map(|a| a.login.as_str()).unwrap_or("")
    }

    pub fn label_names(&self) -> Vec<String> {
        self.labels
            .as_ref()
            .map(|l| l.nodes.iter().map(|n| n.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Reviewers still on the hook, as `login` or `org/team-slug`.
    pub fn requested_reviewers(&self) -> Vec<String> {
        self.review_requests
            .as_ref()
            .map(|rr| {
                rr.nodes
                    .iter()
                    .filter_map(|n| n.requested_reviewer.as_ref())
                    .filter_map(|r| match (&r.login, &r.slug) {
                        (Some(login), _) => Some(login.clone()),
                        (None, Some(slug)) => {
                            let org = self
                                .repository
                                .name_with_owner
                                .split('/')
                                .next()
                                .unwrap_or("");
                            Some(format!("{org}/{slug}"))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn review_list(&self) -> &[RawReview] {
        self.reviews.as_ref().map(|r| r.nodes.as_slice()).unwrap_or(&[])
    }

    pub fn head_oid(&self) -> String {
        self.commits
            .as_ref()
            .and_then(|c| c.nodes.first())
            .map(|c| c.commit.oid.clone())
            .unwrap_or_default()
    }

    pub fn check_contexts(&self) -> &[RawContext] {
        self.commits
            .as_ref()
            .and_then(|c| c.nodes.first())
            .and_then(|c| c.commit.status_check_rollup.as_ref())
            .map(|r| r.contexts.nodes.as_slice())
            .unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Actor {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub name_with_owner: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeList<T> {
    #[serde(default = "Vec::new")]
    pub nodes: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    #[serde(default)]
    pub requested_reviewer: Option<Reviewer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reviewer {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawReview {
    pub state: String,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub author: Option<Actor>,
}

impl RawReview {
    pub fn author_login(&self) -> &str {
        self.author.as_ref().map(|a| a.login.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitNode {
    pub commit: Commit,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub oid: String,
    #[serde(default)]
    pub status_check_rollup: Option<Rollup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rollup {
    pub contexts: NodeList<RawContext>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawContext {
    /// CheckRun name.
    #[serde(default)]
    pub name: Option<String>,
    /// StatusContext name.
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    /// StatusContext verdict.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

impl RawContext {
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.context.clone())
            .unwrap_or_else(|| "check".into())
    }

    /// A check run is `SUCCESS`/`FAILURE`/… once finished. While it is still
    /// running `conclusion` is an **empty string**, not null — so a plain
    /// `conclusion // status` picks the wrong branch. Fall back explicitly.
    pub fn verdict(&self) -> &str {
        if let Some(c) = self.conclusion.as_deref() {
            if !c.is_empty() {
                return c;
            }
        }
        if let Some(s) = self.state.as_deref() {
            if !s.is_empty() {
                return s;
            }
        }
        self.status.as_deref().unwrap_or("PENDING")
    }

    pub fn finished_at(&self) -> Option<DateTime<Utc>> {
        self.completed_at.or(self.created_at)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    pub repository: Repo,
}
