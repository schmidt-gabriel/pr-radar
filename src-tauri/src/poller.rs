//! The 60s poll: three searches, one derived snapshot, notifications for what
//! changed since last time.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::derive;
use crate::github::{Github, Viewer};
use crate::model::{Event, EventKind, Snapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Org whose `Team Review - READY` queue to watch.
    pub org: String,
    pub label: String,
    pub poll_seconds: u64,
    pub notify: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            org: "edsights".into(),
            label: "Team Review - READY".into(),
            poll_seconds: 60,
            notify: true,
        }
    }
}

impl Config {
    pub fn load(dir: &PathBuf) -> Self {
        std::fs::read_to_string(dir.join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &PathBuf) {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join("config.json"), json);
        }
    }
}

pub struct Poller {
    gh: Github,
    pub viewer: Viewer,
    pub config: Config,
}

impl Poller {
    pub async fn new(config: Config) -> Result<Self> {
        let gh = Github::discover().await?;
        let viewer = gh.viewer(&config.org).await?;
        Ok(Self { gh, viewer, config })
    }

    pub async fn poll(&self) -> Result<Snapshot> {
        let now = Utc::now();

        let mine_q = "is:pr is:open author:@me".to_string();
        let queue_q = format!(
            "is:pr is:open org:{} label:\"{}\"",
            self.config.org, self.config.label
        );
        let merged_q = "is:pr author:@me is:merged sort:updated-desc".to_string();

        // Independent searches — run them concurrently so a poll is one round
        // trip's worth of latency, not three.
        let (mine_raw, queue_raw, merged_raw) = tokio::try_join!(
            self.gh.search_prs(&mine_q, 100),
            self.gh.search_prs(&queue_q, 100),
            self.gh.search_closed(&merged_q, 10),
        )?;

        let mine = derive::derive_mine(&mine_raw, now);
        let queue_result = derive::derive_queue(&queue_raw, &self.viewer, now);
        let events = derive::derive_events(&mine_raw, &queue_raw, &self.viewer, now);

        Ok(Snapshot {
            viewer: self.viewer.login.clone(),
            avatar_url: self.viewer.avatar_url.clone(),
            teams: self.viewer.teams.clone(),
            org: self.config.org.clone(),
            fetched_at: now,
            mine_counts: derive::count_mine(&mine),
            mine,
            merged: derive::derive_merged(&merged_raw, now),
            queue_counts: derive::count_queue(&queue_result.items),
            queue: queue_result.items,
            hidden: queue_result.hidden,
            events,
        })
    }
}

// ---------------------------------------------------------------------------
// Notification bookkeeping
// ---------------------------------------------------------------------------

/// Which events have already been surfaced. Persisted so relaunching the app
/// does not re-announce a CI failure you saw yesterday.
#[derive(Default, Serialize, Deserialize)]
pub struct SeenStore {
    ids: Vec<String>,
}

impl SeenStore {
    fn path(dir: &PathBuf) -> PathBuf {
        dir.join("seen-events.json")
    }

    pub fn load(dir: &PathBuf) -> (HashSet<String>, bool) {
        match std::fs::read_to_string(Self::path(dir))
            .ok()
            .and_then(|s| serde_json::from_str::<SeenStore>(&s).ok())
        {
            Some(store) => (store.ids.into_iter().collect(), true),
            // No file: first ever launch. The caller seeds silently rather than
            // firing a notification for every open PR at once.
            None => (HashSet::new(), false),
        }
    }

    pub fn save(dir: &PathBuf, seen: &HashSet<String>) {
        let _ = std::fs::create_dir_all(dir);
        // Unbounded growth would make this file creep; the horizon on events is
        // 14 days, so a few hundred ids is far more than enough.
        let ids: Vec<String> = seen.iter().take(1000).cloned().collect();
        if let Ok(json) = serde_json::to_string(&SeenStore { ids }) {
            let _ = std::fs::write(Self::path(dir), json);
        }
    }
}

/// Events worth interrupting someone for: their PR broke, their PR was
/// approved or rejected, or someone asked them to review.
pub fn is_notifiable(e: &Event) -> bool {
    match e.kind {
        EventKind::CiFail | EventKind::Approved | EventKind::ChangesRequested => e.mine,
        EventKind::ReviewRequested => !e.mine,
        _ => false,
    }
}

pub fn notification_body(e: &Event) -> (String, String) {
    let title = match e.kind {
        EventKind::CiFail => format!("CI failed · {}", e.slug),
        EventKind::Approved => format!("Approved · {}", e.slug),
        EventKind::ChangesRequested => format!("Changes requested · {}", e.slug),
        EventKind::ReviewRequested => format!("Review requested · {}", e.slug),
        _ => e.slug.clone(),
    };
    let body = if e.detail.is_empty() {
        format!("“{}”", e.title)
    } else {
        format!("{} — “{}”", e.detail, e.title)
    };
    (title, body)
}
