//! Run one poll against live GitHub and print the derived snapshot.
//!
//!     cargo run --example snapshot           # human-readable summary
//!     cargo run --example snapshot -- --json # the exact payload the UI receives
//!
//! Useful for checking the derivation rules against real data without launching
//! the app, and for capturing a fixture for the browser preview.

use app_lib::model::{MineBucket, QueueBucket};
use app_lib::poller::{Config, Poller};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = std::env::args().any(|a| a == "--json");

    let config = Config::default();
    let poller = Poller::new(config).await?;
    let snap = poller.poll().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
        return Ok(());
    }

    println!("viewer   {} (teams: {})", snap.viewer, snap.teams.join(", "));
    println!("org      {}", snap.org);
    println!();

    println!(
        "MINE — {} open · {} blocked, {} ready, {} waiting, {} draft",
        snap.mine.len(),
        snap.mine_counts.blocked,
        snap.mine_counts.ready,
        snap.mine_counts.waiting,
        snap.mine_counts.draft
    );
    for pr in &snap.mine {
        let bucket = match pr.bucket {
            MineBucket::Blocked => "BLOCKED",
            MineBucket::Ready => "READY  ",
            MineBucket::Waiting => "WAITING",
            MineBucket::Draft => "DRAFT  ",
        };
        println!(
            "  {bucket} {:26} {:>4}  ci={:8} {:32} {}",
            pr.slug,
            pr.age,
            format!("{:?}", pr.ci.state).to_lowercase(),
            pr.review_text,
            truncate(&pr.title, 48)
        );
    }

    println!();
    println!(
        "QUEUE — {} need a human · hidden: {} mine, {} already reviewed, {} already approved",
        snap.queue.len(),
        snap.hidden.mine,
        snap.hidden.already_reviewed,
        snap.hidden.already_approved
    );
    for pr in &snap.queue {
        let bucket = match pr.bucket {
            QueueBucket::Requested => "ASKED-OF-YOU",
            QueueBucket::NoApproval => "NO-APPROVAL ",
            QueueBucket::Partial => "PARTIAL     ",
        };
        println!(
            "  {bucket} {:34} {:>4}  {:20} {:10} {}",
            pr.slug,
            pr.age,
            pr.author,
            pr.ticket.clone().unwrap_or_else(|| "—".into()),
            truncate(&pr.title, 46)
        );
        if let Some(note) = &pr.sibling_note {
            println!("               ↳ {note}");
        }
    }

    println!();
    println!("EVENTS — {} in the last 14 days", snap.events.len());
    for e in snap.events.iter().take(12) {
        println!(
            "  {}  {:18} {:22} {:26} {}",
            e.at.format("%b %d %H:%M"),
            format!("{:?}", e.kind).to_lowercase(),
            e.slug,
            truncate(&e.headline, 24),
            truncate(&e.detail, 34)
        );
    }

    println!();
    println!("MERGED — last {}", snap.merged.len());
    for pr in snap.merged.iter().take(5) {
        println!("  {:28} {:>4}  {}", pr.slug, pr.age, truncate(&pr.title, 52));
    }

    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}
