//! Raw GitHub responses in, the state all three views read out.
//!
//! This is the single place the two skills' rules are encoded. The popover, the
//! triage window and the timeline never re-derive anything — they only render
//! what comes out of here, which is what keeps them consistent.

use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::github::{ClosedPr, RawContext, RawPr, RawReview, Viewer};
use crate::model::*;

/// Reviews from these accounts never count as approval. CodeRabbit and Copilot
/// comment at length but do not satisfy branch protection, and treating them as
/// approvals would silently empty the review queue.
const BOTS: &[&str] = &[
    "coderabbitai",
    "copilot-pull-request-reviewer",
    "github-actions",
    "sonarcloud",
];

fn is_bot(login: &str) -> bool {
    let lower = login.to_ascii_lowercase();
    lower.ends_with("[bot]") || BOTS.iter().any(|b| lower == *b || lower == format!("{b}[bot]"))
}

fn ticket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Matches CORE-1234, PLAT-217, SEC-438 in the `CORE-1234 |`, `CORE-1234:`
    // and `fix(CORE-1234):` shapes the team uses.
    RE.get_or_init(|| Regex::new(r"\b([A-Z][A-Z0-9]{1,9}-\d+)\b").unwrap())
}

pub fn parse_ticket(title: &str) -> Option<String> {
    ticket_re()
        .captures(title)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

pub fn age_of(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let d = now.signed_duration_since(then);
    if d.num_minutes() < 1 {
        "now".into()
    } else if d.num_hours() < 1 {
        format!("{}m", d.num_minutes())
    } else if d.num_hours() < 24 {
        format!("{}h", d.num_hours())
    } else {
        format!("{}d", d.num_days())
    }
}

fn duration_of(c: &RawContext) -> String {
    let (Some(start), Some(end)) = (c.started_at, c.completed_at) else {
        return String::new();
    };
    let secs = end.signed_duration_since(start).num_seconds().max(0);
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

// ---------------------------------------------------------------------------
// CI
// ---------------------------------------------------------------------------

fn check_state(verdict: &str) -> CheckState {
    match verdict.to_ascii_uppercase().as_str() {
        "SUCCESS" => CheckState::Pass,
        // A cancelled run is finished and not a red X — treat it like a skip so
        // it cannot pin a PR in "pending" forever.
        "SKIPPED" | "NEUTRAL" | "CANCELLED" => CheckState::Skipped,
        "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => {
            CheckState::Fail
        }
        _ => CheckState::Pending,
    }
}

fn build_checks(contexts: &[RawContext]) -> Vec<Check> {
    let mut checks: Vec<Check> = contexts
        .iter()
        .map(|c| Check {
            name: c.label(),
            state: check_state(c.verdict()),
            duration: duration_of(c),
        })
        .collect();

    // Failures first, then still-running, then the green wall of noise.
    let rank = |s: CheckState| match s {
        CheckState::Fail => 0,
        CheckState::Pending => 1,
        CheckState::Pass => 2,
        CheckState::Skipped => 3,
    };
    checks.sort_by_key(|c| (rank(c.state), c.name.clone()));
    checks
}

fn build_ci(checks: &[Check]) -> Ci {
    let failed = checks.iter().filter(|c| c.state == CheckState::Fail).count();
    let pending = checks
        .iter()
        .filter(|c| c.state == CheckState::Pending)
        .count();
    let passed = checks.iter().filter(|c| c.state == CheckState::Pass).count();

    let state = if checks.is_empty() {
        CiState::None
    } else if failed > 0 {
        CiState::Fail
    } else if pending > 0 {
        CiState::Pending
    } else {
        CiState::Pass
    };

    let text = match state {
        CiState::None => "no checks".to_string(),
        CiState::Fail => {
            let first = checks
                .iter()
                .find(|c| c.state == CheckState::Fail)
                .map(|c| c.name.as_str())
                .unwrap_or("a check");
            if failed > 1 {
                format!("{first} +{} failed", failed - 1)
            } else {
                format!("{first} failed")
            }
        }
        CiState::Pending => {
            let first = checks
                .iter()
                .find(|c| c.state == CheckState::Pending)
                .map(|c| c.name.as_str())
                .unwrap_or("checks");
            format!("{first} running")
        }
        CiState::Pass => "all checks passed".to_string(),
    };

    Ci {
        state,
        text,
        passed,
        failed,
        pending,
        total: checks.len(),
    }
}

// ---------------------------------------------------------------------------
// Reviews
// ---------------------------------------------------------------------------

/// A review that moves the branch-protection needle. `DISMISSED` counts: a
/// dismissal explicitly revokes an earlier approval, so it has to be able to
/// override one.
fn is_verdict(state: &str) -> bool {
    matches!(state, "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED")
}

/// GitHub keeps every review ever submitted. What matters for approval is each
/// human's *latest verdict*, so collapse per author and drop bots.
///
/// A trailing `COMMENTED` must not erase a standing approval — which is exactly
/// what reading only the newest row per author would do, and why the skill warns
/// against trusting `latestReviews` alone.
fn latest_human_reviews(reviews: &[RawReview]) -> HashMap<String, &RawReview> {
    let mut latest: HashMap<String, &RawReview> = HashMap::new();

    for r in reviews {
        let login = r.author_login();
        if login.is_empty() || is_bot(login) {
            continue;
        }
        match latest.get(login) {
            None => {
                latest.insert(login.to_string(), r);
            }
            Some(current) => {
                let replace = match (is_verdict(&r.state), is_verdict(&current.state)) {
                    // A verdict always displaces a bare comment.
                    (true, false) => true,
                    // A comment never displaces a verdict.
                    (false, true) => false,
                    // Same class — the newer one wins.
                    _ => r.submitted_at >= current.submitted_at,
                };
                if replace {
                    latest.insert(login.to_string(), r);
                }
            }
        }
    }

    latest
}

fn approvals_of(reviews: &[RawReview]) -> Vec<String> {
    let mut who: Vec<String> = latest_human_reviews(reviews)
        .into_iter()
        .filter(|(_, r)| r.state == "APPROVED")
        .map(|(login, _)| login)
        .collect();
    who.sort();
    who
}

fn changes_requested_by(reviews: &[RawReview]) -> Vec<String> {
    let mut who: Vec<String> = latest_human_reviews(reviews)
        .into_iter()
        .filter(|(_, r)| r.state == "CHANGES_REQUESTED")
        .map(|(login, _)| login)
        .collect();
    who.sort();
    who
}

/// Has this user already weighed in? Any state counts, including a bare comment
/// — if they have looked at it, it should leave their queue.
fn reviewed_by(reviews: &[RawReview], login: &str) -> bool {
    reviews.iter().any(|r| r.author_login() == login)
}

// ---------------------------------------------------------------------------
// My PRs
// ---------------------------------------------------------------------------

pub fn derive_mine(prs: &[RawPr], now: DateTime<Utc>) -> Vec<MinePr> {
    let mut out: Vec<MinePr> = prs
        .iter()
        .map(|pr| {
            let checks = build_checks(pr.check_contexts());
            let ci = build_ci(&checks);
            let reviews = pr.review_list();
            let approvals = approvals_of(reviews);
            let blockers = changes_requested_by(reviews);
            let decision = pr.review_decision.as_deref().unwrap_or("");
            let conflicting = pr.mergeable.as_deref() == Some("CONFLICTING");

            let (review, review_text) = if !blockers.is_empty() || decision == "CHANGES_REQUESTED" {
                let who = blockers.first().cloned();
                (
                    ReviewState::ChangesRequested,
                    match who {
                        Some(w) => format!("Changes requested · {w}"),
                        None => "Changes requested".to_string(),
                    },
                )
            } else if decision == "APPROVED" {
                (ReviewState::Approved, "Approved".to_string())
            } else if !approvals.is_empty() {
                // `reviewDecision` stays REVIEW_REQUIRED (or empty) until branch
                // protection is satisfied, so an approval here is real but not
                // yet enough.
                (
                    ReviewState::ApprovedNeedsMore,
                    "Approved · needs 1 more".to_string(),
                )
            } else if decision == "REVIEW_REQUIRED" {
                (ReviewState::ReviewRequired, "Review required".to_string())
            } else {
                (ReviewState::None, "No reviewers yet".to_string())
            };

            let bucket = if pr.is_draft {
                MineBucket::Draft
            } else if review == ReviewState::ChangesRequested || ci.state == CiState::Fail {
                MineBucket::Blocked
            } else if decision == "APPROVED" && ci.state == CiState::Pass && !conflicting {
                MineBucket::Ready
            } else {
                MineBucket::Waiting
            };

            MinePr {
                id: pr.id(),
                slug: pr.slug(),
                repo: pr.repository.name_with_owner.clone(),
                number: pr.number,
                url: pr.url.clone(),
                title: pr.title.clone(),
                created_at: pr.created_at,
                age: age_of(pr.created_at, now),
                bucket,
                ci,
                review,
                review_text,
                approved_by: approvals,
                conflicting,
                is_draft: pr.is_draft,
                labels: pr.label_names(),
                checks,
                timeline: pr_timeline(pr, now, true),
            }
        })
        .collect();

    // Needs-attention first, exactly as the report does.
    let rank = |b: MineBucket| match b {
        MineBucket::Blocked => 0,
        MineBucket::Ready => 1,
        MineBucket::Waiting => 2,
        MineBucket::Draft => 3,
    };
    out.sort_by(|a, b| {
        rank(a.bucket)
            .cmp(&rank(b.bucket))
            .then(a.created_at.cmp(&b.created_at))
    });
    out
}

/// Per-PR history for the detail pane, from timestamps we actually have.
fn pr_timeline(pr: &RawPr, now: DateTime<Utc>, mine: bool) -> Vec<TimelineEntry> {
    let opened = if mine {
        "You opened the PR".to_string()
    } else {
        format!("{} opened the PR", pr.author_login())
    };
    let mut entries = vec![TimelineEntry {
        at: pr.created_at,
        age: age_of(pr.created_at, now),
        text: opened,
    }];

    for r in pr.review_list() {
        let Some(at) = r.submitted_at else { continue };
        let who = r.author_login();
        if who.is_empty() || is_bot(who) {
            continue;
        }
        let text = match r.state.as_str() {
            "APPROVED" => format!("{who} approved"),
            "CHANGES_REQUESTED" => format!("{who} requested changes"),
            "COMMENTED" => format!("{who} commented"),
            "DISMISSED" => format!("{who}'s review was dismissed"),
            other => format!("{who} left a {} review", other.to_ascii_lowercase()),
        };
        entries.push(TimelineEntry {
            at,
            age: age_of(at, now),
            text,
        });
    }

    // One line per failing check, plus a single line for the last green run.
    let contexts = pr.check_contexts();
    for c in contexts {
        if check_state(c.verdict()) != CheckState::Fail {
            continue;
        }
        let Some(at) = c.finished_at() else { continue };
        let oid = pr.head_oid();
        let short = oid.chars().take(7).collect::<String>();
        entries.push(TimelineEntry {
            at,
            age: age_of(at, now),
            text: format!("{} failed on {short}", c.label()),
        });
    }

    entries.sort_by_key(|e| e.at);
    collapse_timeline(entries, now)
}

/// The per-PR counterpart to [`collapse_bursts`]. A reviewer leaving nine
/// inline comments in one sitting is one action, and rendering it as nine
/// identical rows buries the rest of the PR's history.
///
/// Entries arrive oldest-first; a fold keeps the run's newest timestamp so the
/// row still answers "when did this last happen".
fn collapse_timeline(entries: Vec<TimelineEntry>, now: DateTime<Utc>) -> Vec<TimelineEntry> {
    const WINDOW: i64 = 30 * 60;

    let mut out: Vec<TimelineEntry> = Vec::with_capacity(entries.len());
    for e in entries {
        let fold = out.last().is_some_and(|prev| {
            split_count(&prev.text).0 == e.text
                && (e.at - prev.at).num_seconds().abs() <= WINDOW
        });

        if fold {
            let prev = out.last_mut().expect("checked above");
            let (base, n) = split_count(&prev.text);
            prev.text = format!("{base} ×{}", n + 1);
            prev.at = e.at;
            prev.age = age_of(e.at, now);
        } else {
            out.push(e);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Review queue
// ---------------------------------------------------------------------------

pub struct QueueResult {
    pub items: Vec<QueuePr>,
    pub hidden: Hidden,
}

pub fn derive_queue(prs: &[RawPr], viewer: &Viewer, now: DateTime<Utc>) -> QueueResult {
    let me = viewer.login.as_str();
    let my_handles: HashSet<String> = std::iter::once(me.to_string())
        .chain(viewer.teams.iter().cloned())
        .collect();

    let mut hidden = Hidden::default();
    // Tickets whose siblings dropped out because they are already approved —
    // used to warn that you are only reviewing half of a change.
    let mut approved_siblings: HashMap<String, Vec<String>> = HashMap::new();
    let mut kept: Vec<QueuePr> = Vec::new();

    for pr in prs {
        // `gh search prs` has no negative-author flag, so "not mine" is a
        // client-side filter here just as it is in the skill.
        if pr.author_login() == me {
            hidden.mine += 1;
            hidden.total += 1;
            continue;
        }

        let reviews = pr.review_list();
        if reviewed_by(reviews, me) {
            hidden.already_reviewed += 1;
            hidden.total += 1;
            continue;
        }

        let approvals = approvals_of(reviews);
        let decision = pr.review_decision.as_deref().unwrap_or("");

        // Enough approval already: an explicit APPROVED decision, or no decision
        // at all (repo without required reviews) plus a real human approval. A
        // PR still marked REVIEW_REQUIRED has not cleared the bar — keep it.
        let satisfied = decision == "APPROVED" || (decision.is_empty() && !approvals.is_empty());
        if satisfied {
            if let Some(t) = parse_ticket(&pr.title) {
                approved_siblings.entry(t).or_default().push(pr.slug());
            }
            hidden.already_approved += 1;
            hidden.total += 1;
            continue;
        }

        let requested = pr.requested_reviewers();
        let asked_of_me = requested.iter().any(|r| my_handles.contains(r));

        let bucket = if asked_of_me {
            QueueBucket::Requested
        } else if approvals.is_empty() {
            QueueBucket::NoApproval
        } else {
            QueueBucket::Partial
        };

        let checks = build_checks(pr.check_contexts());
        let ci = build_ci(&checks);

        kept.push(QueuePr {
            id: pr.id(),
            slug: pr.slug(),
            repo: pr.repository.name_with_owner.clone(),
            number: pr.number,
            url: pr.url.clone(),
            title: pr.title.clone(),
            author: pr.author_login().to_string(),
            created_at: pr.created_at,
            age: age_of(pr.created_at, now),
            bucket,
            ticket: parse_ticket(&pr.title),
            ci,
            approvals,
            requested,
            conflicting: pr.mergeable.as_deref() == Some("CONFLICTING"),
            is_draft: pr.is_draft,
            checks,
            timeline: pr_timeline(pr, now, false),
            sibling_note: None,
        });
    }

    for item in &mut kept {
        if let Some(t) = &item.ticket {
            if let Some(siblings) = approved_siblings.get(t) {
                item.sibling_note = Some(format!(
                    "{} already approved — you are reviewing the other half of {t}",
                    siblings.join(", ")
                ));
            }
        }
    }

    let items = order_queue(kept);
    QueueResult { items, hidden }
}

/// Requested-of-you first (oldest first — those have waited on *you*), then the
/// no-approval bucket newest-first so fresh PRs get caught before they stale,
/// then partial approvals oldest first.
fn order_queue(items: Vec<QueuePr>) -> Vec<QueuePr> {
    let mut requested: Vec<QueuePr> = Vec::new();
    let mut no_approval: Vec<QueuePr> = Vec::new();
    let mut partial: Vec<QueuePr> = Vec::new();

    for i in items {
        match i.bucket {
            QueueBucket::Requested => requested.push(i),
            QueueBucket::NoApproval => no_approval.push(i),
            QueueBucket::Partial => partial.push(i),
        }
    }

    requested.sort_by_key(|p| p.created_at);
    partial.sort_by_key(|p| p.created_at);
    // Newest first: catch fresh PRs before they go stale.
    no_approval.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let mut out = requested;
    out.extend(no_approval);
    out.extend(partial);
    out
}

// ---------------------------------------------------------------------------
// Timeline feed
// ---------------------------------------------------------------------------

/// Every event carries a real GitHub timestamp — nothing is inferred from poll
/// time, so the feed still reads correctly after the app has been closed for a
/// week.
pub fn derive_events(
    mine: &[RawPr],
    queue: &[RawPr],
    viewer: &Viewer,
    now: DateTime<Utc>,
) -> Vec<Event> {
    let horizon = now - Duration::days(14);
    let me = viewer.login.as_str();
    let my_handles: HashSet<String> = std::iter::once(me.to_string())
        .chain(viewer.teams.iter().cloned())
        .collect();

    let mut events: Vec<Event> = Vec::new();

    for (pr, is_mine) in mine
        .iter()
        .map(|p| (p, true))
        .chain(queue.iter().map(|p| (p, false)))
    {
        // The org queue and my own PRs overlap (my labeled PRs appear in both);
        // dedupe by id later.
        let slug = pr.slug();
        let author = pr.author_login().to_string();

        if pr.created_at >= horizon {
            events.push(Event {
                id: format!("{}:opened", pr.id()),
                at: pr.created_at,
                kind: EventKind::Opened,
                headline: if is_mine {
                    "You opened a PR".into()
                } else {
                    format!("{author} opened a PR")
                },
                // The slug on the row already names the repo.
                detail: String::new(),
                slug: slug.clone(),
                title: pr.title.clone(),
                url: pr.url.clone(),
                mine: is_mine,
            });
        }

        for r in pr.review_list() {
            let Some(at) = r.submitted_at else { continue };
            if at < horizon {
                continue;
            }
            let who = r.author_login();
            if who.is_empty() || is_bot(who) {
                continue;
            }
            let (kind, headline, detail) = match r.state.as_str() {
                "APPROVED" => (
                    EventKind::Approved,
                    format!("{who} approved"),
                    if is_mine { "on your PR" } else { "" }.to_string(),
                ),
                "CHANGES_REQUESTED" => (
                    EventKind::ChangesRequested,
                    format!("{who} requested changes"),
                    String::new(),
                ),
                "COMMENTED" if who == me => {
                    (EventKind::Commented, "You commented".into(), String::new())
                }
                _ => continue,
            };
            events.push(Event {
                id: format!("{}:review:{who}:{}", pr.id(), at.timestamp()),
                at,
                kind,
                headline,
                detail,
                slug: slug.clone(),
                title: pr.title.clone(),
                url: pr.url.clone(),
                mine: is_mine,
            });
        }

        // One CI event per PR rather than one per check — 40 green rows per PR
        // would bury everything else.
        let checks = build_checks(pr.check_contexts());
        let ci = build_ci(&checks);
        let contexts = pr.check_contexts();
        let oid = pr.head_oid();
        let short: String = oid.chars().take(7).collect();

        match ci.state {
            CiState::Fail => {
                if let Some(c) = contexts
                    .iter()
                    .filter(|c| check_state(c.verdict()) == CheckState::Fail)
                    .max_by_key(|c| c.finished_at())
                {
                    if let Some(at) = c.finished_at() {
                        if at >= horizon {
                            events.push(Event {
                                id: format!("{}:ci_fail:{oid}", pr.id()),
                                at,
                                kind: EventKind::CiFail,
                                headline: "CI failed".into(),
                                detail: format!("{} on {short}", c.label()),
                                slug: slug.clone(),
                                title: pr.title.clone(),
                                url: pr.url.clone(),
                                mine: is_mine,
                            });
                        }
                    }
                }
            }
            CiState::Pending => {
                if let Some(c) = contexts
                    .iter()
                    .filter(|c| check_state(c.verdict()) == CheckState::Pending)
                    .filter_map(|c| c.started_at.map(|at| (at, c)))
                    .max_by_key(|(at, _)| *at)
                {
                    if c.0 >= horizon {
                        events.push(Event {
                            id: format!("{}:ci_running:{oid}", pr.id()),
                            at: c.0,
                            kind: EventKind::CiRunning,
                            headline: "CI running".into(),
                            detail: c.1.label(),
                            slug: slug.clone(),
                            title: pr.title.clone(),
                            url: pr.url.clone(),
                            mine: is_mine,
                        });
                    }
                }
            }
            CiState::Pass if is_mine => {
                if let Some(at) = contexts.iter().filter_map(|c| c.finished_at()).max() {
                    if at >= horizon {
                        events.push(Event {
                            id: format!("{}:ci_pass:{oid}", pr.id()),
                            at,
                            kind: EventKind::CiPass,
                            headline: "CI green".into(),
                            detail: format!("{} checks passed on {short}", ci.passed),
                            slug: slug.clone(),
                            title: pr.title.clone(),
                            url: pr.url.clone(),
                            mine: is_mine,
                        });
                    }
                }
            }
            _ => {}
        }

        // "Review requested from you" has no timestamp on the request itself;
        // the PR's last update is the closest honest anchor.
        if !is_mine && pr.requested_reviewers().iter().any(|r| my_handles.contains(r)) {
            if pr.updated_at >= horizon {
                events.push(Event {
                    id: format!("{}:review_requested", pr.id()),
                    at: pr.updated_at,
                    kind: EventKind::ReviewRequested,
                    headline: "Review requested from you".into(),
                    detail: format!("by {author}"),
                    slug: slug.clone(),
                    title: pr.title.clone(),
                    url: pr.url.clone(),
                    mine: false,
                });
            }
        }
    }

    let mut seen = HashSet::new();
    events.retain(|e| seen.insert(e.id.clone()));
    events.sort_by(|a, b| b.at.cmp(&a.at));
    let mut events = collapse_bursts(events);
    events.truncate(120);
    events
}

/// Three review comments posted 11 seconds apart are one action, not three
/// rows. Collapse a run of same-kind events on the same PR into the newest,
/// noting how many were folded in.
fn collapse_bursts(events: Vec<Event>) -> Vec<Event> {
    const WINDOW: i64 = 30 * 60;

    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for e in events {
        let fold = out.last().is_some_and(|prev| {
            prev.kind == e.kind
                && prev.slug == e.slug
                && prev.headline == e.headline
                && (prev.at - e.at).num_seconds().abs() <= WINDOW
        });

        if fold {
            let prev = out.last_mut().expect("checked above");
            let (base, n) = split_count(&prev.detail);
            prev.detail = if base.is_empty() {
                format!("×{}", n + 1)
            } else {
                format!("{base} ×{}", n + 1)
            };
        } else {
            out.push(e);
        }
    }
    out
}

/// Split a detail string back into its text and the fold count appended to it,
/// so repeated folds keep the original detail instead of overwriting it.
fn split_count(detail: &str) -> (&str, usize) {
    match detail.rsplit_once('×') {
        Some((base, n)) => match n.parse::<usize>() {
            Ok(n) => (base.trim_end(), n),
            Err(_) => (detail, 1),
        },
        None => (detail, 1),
    }
}

pub fn derive_merged(prs: &[ClosedPr], now: DateTime<Utc>) -> Vec<MergedPr> {
    let mut out: Vec<MergedPr> = prs
        .iter()
        .map(|p| {
            let at = p.merged_at.or(p.closed_at).unwrap_or(now);
            let short = p
                .repository
                .name_with_owner
                .split('/')
                .next_back()
                .unwrap_or("");
            MergedPr {
                slug: format!("{short}#{}", p.number),
                title: p.title.clone(),
                url: p.url.clone(),
                state: if p.merged_at.is_some() {
                    "merged".into()
                } else {
                    p.state.to_ascii_lowercase()
                },
                age: age_of(at, now),
            }
        })
        .collect();
    out.truncate(10);
    out
}

pub fn count_mine(mine: &[MinePr]) -> MineCounts {
    let mut c = MineCounts::default();
    for p in mine {
        match p.bucket {
            MineBucket::Blocked => c.blocked += 1,
            MineBucket::Ready => c.ready += 1,
            MineBucket::Waiting => c.waiting += 1,
            MineBucket::Draft => c.draft += 1,
        }
    }
    c
}

pub fn count_queue(queue: &[QueuePr]) -> QueueCounts {
    let mut c = QueueCounts::default();
    for p in queue {
        match p.bucket {
            QueueBucket::Requested => c.requested += 1,
            QueueBucket::NoApproval => c.no_approval += 1,
            QueueBucket::Partial => c.partial += 1,
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_ticket_shape_the_team_uses() {
        assert_eq!(
            parse_ticket("CORE-1559 | Sort campaigns by date"),
            Some("CORE-1559".into())
        );
        assert_eq!(
            parse_ticket("CORE-1584: Enforce school active-status"),
            Some("CORE-1584".into())
        );
        assert_eq!(
            parse_ticket("fix(CORE-1616): correct RAG scoring"),
            Some("CORE-1616".into())
        );
        assert_eq!(parse_ticket("PLAT-217 | Add audit logging"), Some("PLAT-217".into()));
        assert_eq!(parse_ticket("Add golangci-lint gate"), None);
    }

    #[test]
    fn bots_never_count_as_reviewers() {
        assert!(is_bot("coderabbitai"));
        assert!(is_bot("CodeRabbitAI"));
        assert!(is_bot("copilot-pull-request-reviewer"));
        assert!(is_bot("dependabot[bot]"));
        assert!(!is_bot("hconrad"));
        assert!(!is_bot("hampsights"));
    }

    fn review(author: &str, state: &str, mins: i64) -> RawReview {
        RawReview {
            state: state.into(),
            submitted_at: Some(Utc::now() - Duration::minutes(mins)),
            author: Some(crate::github::Actor {
                login: author.into(),
            }),
        }
    }

    #[test]
    fn a_comment_after_an_approval_does_not_erase_it() {
        let reviews = vec![
            review("hconrad", "APPROVED", 60),
            review("hconrad", "COMMENTED", 10),
        ];
        assert_eq!(approvals_of(&reviews), vec!["hconrad".to_string()]);
    }

    #[test]
    fn a_later_change_request_overrides_an_earlier_approval() {
        let reviews = vec![
            review("hconrad", "APPROVED", 60),
            review("hconrad", "CHANGES_REQUESTED", 10),
        ];
        assert!(approvals_of(&reviews).is_empty());
        assert_eq!(changes_requested_by(&reviews), vec!["hconrad".to_string()]);
    }

    #[test]
    fn a_dismissal_revokes_the_approval_it_followed() {
        let reviews = vec![
            review("valcantara23", "APPROVED", 60),
            review("valcantara23", "DISMISSED", 10),
        ];
        assert!(approvals_of(&reviews).is_empty());
    }

    #[test]
    fn bot_approval_never_satisfies_the_queue() {
        let reviews = vec![review("coderabbitai", "APPROVED", 5)];
        assert!(approvals_of(&reviews).is_empty());
    }

    #[test]
    fn a_comment_storm_collapses_to_one_row() {
        let now = Utc::now();
        let entry = |mins: i64, text: &str| TimelineEntry {
            at: now - Duration::minutes(mins),
            age: age_of(now - Duration::minutes(mins), now),
            text: text.into(),
        };

        // Oldest first, as pr_timeline produces them.
        let out = collapse_timeline(
            vec![
                entry(600, "You opened the PR"),
                entry(48, "dmarrero commented"),
                entry(47, "dmarrero commented"),
                entry(47, "dmarrero commented"),
                entry(46, "dmarrero commented"),
                entry(5, "hconrad approved"),
            ],
            now,
        );

        let texts: Vec<&str> = out.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "You opened the PR",
                "dmarrero commented ×4",
                "hconrad approved"
            ]
        );
        // The fold keeps the newest timestamp in the run.
        assert_eq!(out[1].at, now - Duration::minutes(46));
    }

    #[test]
    fn distant_repeats_are_not_collapsed() {
        let now = Utc::now();
        let entry = |mins: i64| TimelineEntry {
            at: now - Duration::minutes(mins),
            age: age_of(now - Duration::minutes(mins), now),
            text: "hconrad commented".into(),
        };
        // Two hours apart: separate visits, so separate rows.
        let out = collapse_timeline(vec![entry(180), entry(60)], now);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn running_checks_report_pending_not_pass() {
        // GitHub sends conclusion:"" (not null) while a check is in flight.
        let ctx = RawContext {
            name: Some("lint / eslint".into()),
            context: None,
            status: Some("IN_PROGRESS".into()),
            conclusion: Some(String::new()),
            state: None,
            started_at: None,
            completed_at: None,
            created_at: None,
        };
        let checks = build_checks(&[ctx]);
        let ci = build_ci(&checks);
        assert_eq!(ci.state, CiState::Pending);
        assert_eq!(ci.text, "lint / eslint running");
    }

    #[test]
    fn one_failure_beats_many_passes() {
        let mk = |name: &str, concl: &str| RawContext {
            name: Some(name.into()),
            context: None,
            status: Some("COMPLETED".into()),
            conclusion: Some(concl.into()),
            state: None,
            started_at: None,
            completed_at: None,
            created_at: None,
        };
        let checks = build_checks(&[
            mk("unit", "SUCCESS"),
            mk("pytest / integration", "FAILURE"),
            mk("codeql", "SKIPPED"),
        ]);
        let ci = build_ci(&checks);
        assert_eq!(ci.state, CiState::Fail);
        assert_eq!(ci.text, "pytest / integration failed");
        assert_eq!(ci.passed, 1);
    }
}
