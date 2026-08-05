// 1a — Menu bar popover. The 10×/day glance: what is on fire, what is clear to
// merge, and what is waiting on me to review. Every row opens GitHub.

import { useEffect, useMemo, useState } from "react";
import { hidePopover, open, openMain, useFeed, usePrefs } from "../lib/feed";
import { displayAge, syncedAgo } from "../lib/format";
import { chord, hasMod } from "../lib/platform";
import type { MineBucket, MinePr, QueuePr, Snapshot } from "../lib/types";
import {
  CiBadge,
  Dot,
  Empty,
  ErrorBanner,
  MINE_BUCKET_LABEL,
  QUEUE_BUCKET_TAG,
  RefreshIcon,
  ReviewLabel,
} from "../ui/atoms";

type Tab = "mine" | "review";

const GROUP_ORDER: MineBucket[] = ["blocked", "ready", "waiting", "draft"];

export default function Popover() {
  const { feed, refresh } = useFeed();
  const { prefs } = usePrefs();
  const [tab, setTab] = useState<Tab>("mine");
  const [spinning, setSpinning] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  // Keeps "synced 40s ago" honest without re-fetching anything.
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(t);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = hasMod(e);
      if (mod && e.key === "r") {
        e.preventDefault();
        doRefresh();
      } else if (mod && e.key === "1") {
        e.preventDefault();
        setTab("mine");
      } else if (mod && e.key === "2") {
        e.preventDefault();
        setTab("review");
      } else if (e.key === "Escape") {
        hidePopover();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function doRefresh() {
    setSpinning(true);
    refresh();
    setTimeout(() => setSpinning(false), 900);
  }

  const snap = feed.status === "ready" ? feed.data : null;
  const queue = useMemo(() => visibleQueue(snap, prefs.hideApprovedInQueue), [snap, prefs]);

  return (
    <div className="popover">
      <div className="popover-head">
        <div className="segmented">
          <button aria-selected={tab === "mine"} onClick={() => setTab("mine")}>
            Mine <span className="count">{snap ? snap.mine.length : "–"}</span>
          </button>
          <button aria-selected={tab === "review"} onClick={() => setTab("review")}>
            To review <span className="count">{snap ? queue.length : "–"}</span>
          </button>
        </div>
        <div className="meta">
          <span className="synced">
            {snap ? syncedAgo(snap.fetchedAt, now) : "connecting…"}
          </span>
          <button className="icon-btn" onClick={doRefresh} title={`Refresh (${chord("R")})`}>
            <RefreshIcon spinning={spinning || feed.status === "loading"} />
          </button>
        </div>
      </div>

      <div className="popover-body scroll">
        {feed.status === "error" && <ErrorBanner message={feed.data} />}
        {feed.status === "loading" && (
          <Empty glyph="◌">
            <span className="breathing">Talking to GitHub…</span>
          </Empty>
        )}
        {snap && tab === "mine" && <MineTab snap={snap} relative={prefs.relativeTime} />}
        {snap && tab === "review" && (
          <ReviewTab snap={snap} queue={queue} relative={prefs.relativeTime} />
        )}
      </div>

      <div className="popover-foot">
        <button onClick={() => openMain("triage")}>
          {chord("R")} refresh · {chord("1")}/{chord("2")} tabs
        </button>
        <span>{snap?.viewer ?? ""}</span>
      </div>
    </div>
  );
}

function visibleQueue(snap: Snapshot | null, hideApproved: boolean): QueuePr[] {
  if (!snap) return [];
  return hideApproved ? snap.queue.filter((p) => p.bucket !== "partial") : snap.queue;
}

function MineTab({ snap, relative }: { snap: Snapshot; relative: boolean }) {
  const groups = GROUP_ORDER.map((bucket) => ({
    bucket,
    items: snap.mine.filter((p) => p.bucket === bucket),
  })).filter((g) => g.items.length > 0);

  if (snap.mine.length === 0) {
    return (
      <Empty glyph="◎">
        No open PRs of yours.
        <br />
        Nothing to chase today.
      </Empty>
    );
  }

  return (
    <>
      <div className="stat-row">
        <div className="stat-tile blocked">
          <div className="n">{snap.mineCounts.blocked}</div>
          <div className="k">blocked on you</div>
        </div>
        <div className="stat-tile ready">
          <div className="n">{snap.mineCounts.ready}</div>
          <div className="k">ready to merge</div>
        </div>
        <div className="stat-tile">
          <div className="n">{snap.mineCounts.waiting}</div>
          <div className="k">waiting</div>
        </div>
      </div>

      {groups.map((g) => (
        <div key={g.bucket}>
          <div className="group-head">
            <Dot state={g.bucket} />
            <span className="label">{MINE_BUCKET_LABEL[g.bucket]}</span>
            <span className="n">{g.items.length}</span>
          </div>
          {g.items.map((pr) => (
            <MineRow key={pr.id} pr={pr} relative={relative} />
          ))}
        </div>
      ))}
    </>
  );
}

function MineRow({ pr, relative }: { pr: MinePr; relative: boolean }) {
  return (
    <button className="pop-row" onClick={() => open(pr.url)} title={pr.title}>
      <div className="top">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="title">{pr.title}</div>
          <div className="facts">
            <span className="slug">{pr.slug}</span>
            <span className="sep" />
            <CiBadge ci={pr.ci} />
            <span className="sep" />
            <ReviewLabel review={pr.review} text={pr.reviewText} />
            {pr.conflicting && (
              <>
                <span className="sep" />
                <span className="s-fail" style={{ fontSize: 11 }}>
                  conflicts
                </span>
              </>
            )}
          </div>
        </div>
        <span className="age">{displayAge(pr.age, pr.createdAt, relative)}</span>
      </div>
    </button>
  );
}

function ReviewTab({
  snap,
  queue,
  relative,
}: {
  snap: Snapshot;
  queue: QueuePr[];
  relative: boolean;
}) {
  if (queue.length === 0) {
    return (
      <Empty glyph="✓">
        Nothing waiting on you.
        <br />
        {snap.hidden.total > 0 && `${snap.hidden.total} labeled PRs are already handled.`}
      </Empty>
    );
  }

  return (
    <>
      <div className="queue-note">
        {queue.length} need a human ·{" "}
        <b>{snap.hidden.alreadyApproved + snap.hidden.alreadyReviewed} hidden</b> (already
        approved or reviewed by you)
      </div>
      {queue.map((pr) => (
        <button
          key={pr.id}
          className="pop-row"
          onClick={() => open(pr.url)}
          title={pr.siblingNote ?? pr.title}
        >
          <div className="top">
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <span className={`bucket-tag s-${pr.bucket}`}>
                  {QUEUE_BUCKET_TAG[pr.bucket]}
                </span>
                {pr.ticket && <span className="ticket">{pr.ticket}</span>}
              </div>
              <div className="title" style={{ marginTop: 3 }}>
                {pr.title}
              </div>
              <div className="facts">
                <span className="slug">{pr.slug}</span>
                <span className="author">{pr.author}</span>
                <CiBadge ci={pr.ci} />
              </div>
            </div>
            <span className="age">{displayAge(pr.age, pr.createdAt, relative)}</span>
          </div>
        </button>
      ))}
    </>
  );
}
