//! Everything the frontend renders. One snapshot, three views.
//!
//! Deliberately semantic rather than presentational: the payload carries states
//! (`fail`, `blocked`, `no_approval`), never colors. Colors live in the design
//! tokens on the React side so the popover, the triage window and the timeline
//! cannot drift apart.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiState {
    Pass,
    Fail,
    Pending,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MineBucket {
    /// Changes requested, or CI is failing. Blocked on the user.
    Blocked,
    /// Approved, green, mergeable.
    Ready,
    /// Waiting on someone else — including "approved but branch protection
    /// wants another".
    Waiting,
    Draft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    ChangesRequested,
    Approved,
    /// Has a human approval but the branch protection bar is not met yet.
    ApprovedNeedsMore,
    ReviewRequired,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueBucket {
    /// The user, or one of their teams, is on the review request.
    Requested,
    /// Nobody has approved yet — blocks the author hardest.
    NoApproval,
    /// Has an approval, still short of branch protection.
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Pass,
    Fail,
    Pending,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ci {
    pub state: CiState,
    /// Human summary: "all checks passed", "pytest / integration failed".
    pub text: String,
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    /// Wall-clock duration, e.g. "4m12s". Empty when the check never ran.
    pub duration: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub at: DateTime<Utc>,
    pub age: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinePr {
    pub id: String,
    pub slug: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub age: String,
    pub bucket: MineBucket,
    pub ci: Ci,
    pub review: ReviewState,
    pub review_text: String,
    pub approved_by: Vec<String>,
    pub conflicting: bool,
    pub is_draft: bool,
    pub labels: Vec<String>,
    pub checks: Vec<Check>,
    pub timeline: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePr {
    pub id: String,
    pub slug: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub age: String,
    pub bucket: QueueBucket,
    pub ticket: Option<String>,
    pub ci: Ci,
    pub approvals: Vec<String>,
    pub requested: Vec<String>,
    pub conflicting: bool,
    pub is_draft: bool,
    pub checks: Vec<Check>,
    pub timeline: Vec<TimelineEntry>,
    /// Set when a sibling PR on the same ticket was filtered out, so a short
    /// list never hides the fact that you are reviewing half a change.
    pub sibling_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedPr {
    pub slug: String,
    pub title: String,
    pub url: String,
    pub state: String,
    pub age: String,
}

/// Why the queue is shorter than the raw label count — reported so a short list
/// never reads as an empty queue.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hidden {
    pub total: usize,
    pub already_reviewed: usize,
    pub already_approved: usize,
    pub mine: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    CiFail,
    CiPass,
    CiRunning,
    Approved,
    ChangesRequested,
    Commented,
    Opened,
    ReviewRequested,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Stable across polls — used to dedupe notifications between launches.
    pub id: String,
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub headline: String,
    pub detail: String,
    pub slug: String,
    pub title: String,
    pub url: String,
    pub mine: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MineCounts {
    pub blocked: usize,
    pub ready: usize,
    pub waiting: usize,
    pub draft: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueCounts {
    pub requested: usize,
    pub no_approval: usize,
    pub partial: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub viewer: String,
    pub avatar_url: String,
    pub teams: Vec<String>,
    pub org: String,
    pub fetched_at: DateTime<Utc>,
    pub mine: Vec<MinePr>,
    pub mine_counts: MineCounts,
    pub merged: Vec<MergedPr>,
    pub queue: Vec<QueuePr>,
    pub queue_counts: QueueCounts,
    pub hidden: Hidden,
    pub events: Vec<Event>,
}

/// What the UI is told while a poll is in flight or after one fails.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status", content = "data")]
pub enum Feed {
    Loading,
    Ready(Box<Snapshot>),
    Error(String),
}
