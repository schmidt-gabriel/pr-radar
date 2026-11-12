//! Thin GitHub GraphQL client.
//!
//! The two skills this app replaces (`my-prs`, `prs-to-review`) shell out to `gh`
//! once per PR, which is fine for a one-off report but not for a 60s poller —
//! 30 labeled PRs meant 30 subprocesses and 30 round trips. The same fields are
//! available from a GraphQL search, so each poll is a handful of requests: one
//! per 25-PR page of each search.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const USER_AGENT: &str = "pr-radar/0.1";

/// Search results per request. GitHub aborts a GraphQL query that runs past
/// ~10s; 100 PRs with their nested reviews, threads and checks took 8-11s and
/// intermittently came back as a 502. 25 per page keeps each request well
/// under the limit.
const SEARCH_PAGE_SIZE: u32 = 25;
/// One initial attempt plus one retry for transient gateway errors.
const MAX_ATTEMPTS: u32 = 2;
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
/// How much of an unexpected response body to quote in an error message.
const BODY_SNIPPET_CHARS: usize = 200;

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

    async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        vars: serde_json::Value,
    ) -> Result<T> {
        let payload = json!({ "query": query, "variables": vars });

        // GitHub's edge answers a slow query with an nginx HTML `502 Bad
        // Gateway` rather than JSON. These are usually transient, so one retry
        // after a short pause recovers most of them without hammering the API.
        let mut attempt = 0;
        let (status, body) = loop {
            attempt += 1;
            let resp = self
                .client
                .post(GRAPHQL_URL)
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await
                .context("GitHub GraphQL request failed")?;

            let status = resp.status();
            // Read as text, not `.json()`: decoding first turns an HTML error
            // page into "expected value at line 1 column 1", hiding the status.
            let body = resp
                .text()
                .await
                .context("could not read GitHub's response body")?;

            if attempt < MAX_ATTEMPTS && is_retryable(status) {
                log::warn!("GitHub returned {status}; retrying once");
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
            break (status, body);
        };

        let data = parse_graphql_body(status, &body)?;
        Ok(serde_json::from_value(data)?)
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
    ///
    /// Fetched in pages of `SEARCH_PAGE_SIZE` until `limit` results have been
    /// seen or the search runs out, so no single request risks GitHub's
    /// GraphQL timeout.
    pub async fn search_prs(&self, q: &str, limit: u32) -> Result<Vec<RawPr>> {
        #[derive(Deserialize)]
        struct Resp {
            search: SearchPage,
        }

        let nodes = paginate(limit, SEARCH_PAGE_SIZE, |first, after| async move {
            let resp: Resp = self
                .graphql(
                    PR_SEARCH_QUERY,
                    json!({ "q": q, "first": first, "after": after }),
                )
                .await?;
            Ok(resp.search)
        })
        .await?;

        // Search returns a union; non-PR nodes come back as `{}`. Skip anything
        // that does not parse rather than failing the whole poll.
        Ok(nodes
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

        let resp: Resp = self
            .graphql(query, json!({ "q": q, "limit": limit }))
            .await?;
        Ok(resp
            .search
            .nodes
            .into_iter()
            .filter_map(|n| serde_json::from_value::<ClosedPr>(n).ok())
            .filter(|p| p.number > 0)
            .collect())
    }
}

/// One page of a GraphQL search connection.
#[derive(Debug, Deserialize)]
struct SearchPage {
    #[serde(default)]
    nodes: Vec<serde_json::Value>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

/// Drive a cursor-paged search until `limit` nodes are collected or the
/// connection is exhausted. `fetch(first, after)` returns one page.
///
/// Stops on an empty page or a missing cursor even if `hasNextPage` claims
/// otherwise, so a misbehaving response cannot spin the poller forever.
async fn paginate<F, Fut>(
    limit: u32,
    page_size: u32,
    mut fetch: F,
) -> Result<Vec<serde_json::Value>>
where
    F: FnMut(u32, Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<SearchPage>>,
{
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut after: Option<String> = None;

    while (out.len() as u32) < limit {
        let first = (limit - out.len() as u32).min(page_size.max(1));
        let page = fetch(first, after.take()).await?;
        let got = page.nodes.len();
        out.extend(page.nodes);

        match (page.page_info.has_next_page, page.page_info.end_cursor) {
            (true, Some(cursor)) if got > 0 => after = Some(cursor),
            _ => break,
        }
    }

    out.truncate(limit as usize);
    Ok(out)
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 502..=504)
}

/// The first `BODY_SNIPPET_CHARS` characters of a body, whitespace collapsed,
/// so an HTML error page reads as one short line in the UI.
fn body_snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > BODY_SNIPPET_CHARS {
        let cut: String = collapsed.chars().take(BODY_SNIPPET_CHARS).collect();
        format!("{cut}…")
    } else {
        collapsed
    }
}

/// Turn a raw GraphQL HTTP response into its `data` value, or an error that
/// names what actually went wrong (status, non-JSON body, GraphQL errors).
fn parse_graphql_body(status: reqwest::StatusCode, body: &str) -> Result<serde_json::Value> {
    if !status.is_success() {
        let snippet = body_snippet(body);
        return Err(if snippet.is_empty() {
            anyhow!("GitHub returned {status}")
        } else {
            anyhow!("GitHub returned {status}: {snippet}")
        });
    }

    let mut parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        anyhow!(
            "GitHub returned a non-JSON response ({status}): {e}: {}",
            body_snippet(body)
        )
    })?;

    // GraphQL reports partial failures with HTTP 200 and an `errors` array.
    if let Some(errors) = parsed.get("errors").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .map(|s| s.to_string())
                .collect();
            return Err(anyhow!("GitHub GraphQL error: {}", msgs.join("; ")));
        }
    }

    parsed
        .get_mut("data")
        .map(serde_json::Value::take)
        .ok_or_else(|| anyhow!("GitHub response had no data"))
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

// Nested page sizes are a trade between completeness and staying under GitHub's
// GraphQL timeout. Paging the search to 25 PRs is what bounds the cost; these
// are sized from the live review queue (Nov 2026: up to 65 reviews, 48
// threads and 34 check contexts on a single PR). Known truncation risks:
// - `reviews(last: 50)`: the newest 50, which is what verdicts depend on (the
//   old `first: 60` dropped the newest instead). A reviewer whose only verdict
//   is older than that window loses it, and the viewer's own review older than
//   the window will not hide a PR from their queue.
// - `reviewThreads(last: 100)`: an unresolved thread older than the newest
//   100 is missed, so the "unresolved conversations" blocker can under-report.
// - `contexts(first: 50)`: a commit with more than 50 checks can hide a
//   failing or pending one from the CI rollup, which is derived from these.
const PR_SEARCH_QUERY: &str = r#"
query($q: String!, $first: Int!, $after: String) {
  search(query: $q, type: ISSUE, first: $first, after: $after) {
    pageInfo { hasNextPage endCursor }
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
        reviews(last: 50) {
          nodes { state submittedAt author { login } }
        }
        reviewThreads(last: 100) {
          nodes { isResolved }
        }
        commits(last: 1) {
          nodes {
            commit {
              oid
              statusCheckRollup {
                state
                contexts(first: 50) {
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
    pub review_threads: Option<NodeList<RawReviewThread>>,
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
        self.reviews
            .as_ref()
            .map(|r| r.nodes.as_slice())
            .unwrap_or(&[])
    }

    pub fn has_unresolved_review_threads(&self) -> bool {
        self.review_threads
            .as_ref()
            .is_some_and(|threads| threads.nodes.iter().any(|thread| !thread.is_resolved))
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawReviewThread {
    #[serde(default)]
    pub is_resolved: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use std::cell::RefCell;

    const NGINX_502: &str = "<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\r\n<center><h1>502 Bad Gateway</h1></center>\r\n<hr><center>nginx</center>\r\n</body>\r\n</html>\r\n";

    #[test]
    fn non_2xx_html_names_the_status_not_a_decode_error() {
        let err = parse_graphql_body(StatusCode::BAD_GATEWAY, NGINX_502).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.starts_with("GitHub returned 502 Bad Gateway: <html> <head><title>502"),
            "{msg}"
        );
        assert!(
            !msg.contains('\n') && !msg.contains("expected value"),
            "{msg}"
        );
    }

    #[test]
    fn long_bodies_are_truncated() {
        let body = "x ".repeat(500);
        let msg = format!(
            "{:#}",
            parse_graphql_body(StatusCode::SERVICE_UNAVAILABLE, &body).unwrap_err()
        );
        let snippet = msg.split(": ").nth(1).unwrap();
        assert_eq!(snippet.chars().count(), BODY_SNIPPET_CHARS + 1);
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn non_2xx_with_empty_body_still_reads_cleanly() {
        let msg = format!(
            "{:#}",
            parse_graphql_body(StatusCode::GATEWAY_TIMEOUT, "  ").unwrap_err()
        );
        assert_eq!(msg, "GitHub returned 504 Gateway Timeout");
    }

    #[test]
    fn ok_status_with_non_json_body_quotes_a_snippet() {
        let msg = format!(
            "{:#}",
            parse_graphql_body(StatusCode::OK, "<html>oops</html>").unwrap_err()
        );
        assert!(
            msg.starts_with("GitHub returned a non-JSON response (200 OK)"),
            "{msg}"
        );
        assert!(msg.ends_with("<html>oops</html>"), "{msg}");
    }

    #[test]
    fn graphql_errors_and_data_are_handled() {
        let err = parse_graphql_body(
            StatusCode::OK,
            r#"{"errors":[{"message":"a"},{"message":"b"}]}"#,
        )
        .unwrap_err();
        assert_eq!(format!("{err:#}"), "GitHub GraphQL error: a; b");

        let data = parse_graphql_body(StatusCode::OK, r#"{"data":{"x":1}}"#).unwrap();
        assert_eq!(data, json!({ "x": 1 }));
    }

    #[test]
    fn only_gateway_errors_are_retried() {
        assert!(is_retryable(StatusCode::BAD_GATEWAY));
        assert!(is_retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable(StatusCode::GATEWAY_TIMEOUT));
        assert!(!is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_retryable(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable(StatusCode::OK));
    }

    /// A fake search with `total` results, recording each `(first, after)`.
    fn fake_search(
        total: usize,
        calls: &RefCell<Vec<(u32, Option<String>)>>,
        first: u32,
        after: Option<String>,
    ) -> Result<SearchPage> {
        calls.borrow_mut().push((first, after.clone()));
        let start: usize = after.map(|c| c.parse().unwrap()).unwrap_or(0);
        let end = (start + first as usize).min(total);
        Ok(SearchPage {
            nodes: (start..end).map(|i| json!(i)).collect(),
            page_info: PageInfo {
                has_next_page: end < total,
                end_cursor: Some(end.to_string()),
            },
        })
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn paging_stops_when_the_search_runs_out() {
        let calls = RefCell::new(Vec::new());
        let out = run(paginate(100, 25, |first, after| {
            std::future::ready(fake_search(57, &calls, first, after))
        }))
        .unwrap();
        assert_eq!(out.len(), 57);
        assert_eq!(
            *calls.borrow(),
            vec![(25, None), (25, Some("25".into())), (25, Some("50".into()))]
        );
    }

    #[test]
    fn paging_stops_at_the_limit_and_shrinks_the_last_page() {
        let calls = RefCell::new(Vec::new());
        let out = run(paginate(30, 25, |first, after| {
            std::future::ready(fake_search(500, &calls, first, after))
        }))
        .unwrap();
        assert_eq!(out.len(), 30);
        assert_eq!(*calls.borrow(), vec![(25, None), (5, Some("25".into()))]);
    }

    #[test]
    fn paging_does_not_spin_on_a_bogus_has_next_page() {
        let calls = RefCell::new(0);
        let out = run(paginate(100, 25, |_, _| {
            *calls.borrow_mut() += 1;
            std::future::ready(Ok(SearchPage {
                nodes: vec![],
                page_info: PageInfo {
                    has_next_page: true,
                    end_cursor: Some("same".into()),
                },
            }))
        }))
        .unwrap();
        assert!(out.is_empty());
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn paging_propagates_errors() {
        let err = run(paginate(100, 25, |_, _| {
            std::future::ready(Err::<SearchPage, _>(anyhow!("GitHub returned 502")))
        }))
        .unwrap_err();
        assert_eq!(err.to_string(), "GitHub returned 502");
    }
}
