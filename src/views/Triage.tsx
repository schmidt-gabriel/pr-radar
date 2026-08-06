// 1b — Desktop triage window. List + detail, the "sit down and clear it" view.

import { useEffect, useMemo, useState } from "react";
import { open } from "../lib/feed";
import { approvedByLabel } from "../lib/format";
import type {
  Check,
  MineBucket,
  MinePr,
  Prefs,
  QueueBucket,
  QueuePr,
  Snapshot,
  TimelineEntry,
} from "../lib/types";
import {
  CiBadge,
  Dot,
  Empty,
  MINE_BUCKET_LABEL,
  QUEUE_BUCKET_LABEL,
  ReviewLabel,
} from "../ui/atoms";

type Selection =
  | { side: "mine"; bucket: MineBucket | "all" }
  | { side: "merged" }
  | { side: "queue"; bucket: QueueBucket | "all" };

const MINE_RAIL: MineBucket[] = ["blocked", "ready", "waiting", "draft"];
const QUEUE_RAIL: QueueBucket[] = ["requested", "no_approval", "partial"];

/**
 * `urgency` keeps the order the backend derived: blocked first for your PRs,
 * asked-of-you first for the queue. `oldest` throws that away and sorts purely
 * by age, for when you want to clear the things that have been sitting longest.
 */
type Sort = "urgency" | "oldest";

export default function Triage({ snap, prefs }: { snap: Snapshot; prefs: Prefs }) {
  const [selection, setSelection] = useState<Selection>({ side: "mine", bucket: "all" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sort, setSort] = useState<Sort>("urgency");

  const queue = useMemo(
    () => (prefs.hideApprovedInQueue ? snap.queue.filter((p) => p.bucket !== "partial") : snap.queue),
    [snap.queue, prefs.hideApprovedInQueue],
  );

  const rows: (MinePr | QueuePr)[] = useMemo(() => {
    let base: (MinePr | QueuePr)[];
    if (selection.side === "merged") {
      base = [];
    } else if (selection.side === "mine") {
      base =
        selection.bucket === "all"
          ? snap.mine
          : snap.mine.filter((p) => p.bucket === selection.bucket);
    } else {
      base =
        selection.bucket === "all"
          ? queue
          : queue.filter((p) => p.bucket === selection.bucket);
    }

    // createdAt is ISO-8601 in UTC, so a string compare is a date compare.
    if (sort === "oldest") {
      return [...base].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
    }
    return base;
  }, [selection, snap.mine, queue, sort]);

  // Keep a sensible selection as data churns underneath.
  useEffect(() => {
    if (rows.length === 0) {
      setSelectedId(null);
    } else if (!rows.some((r) => r.id === selectedId)) {
      setSelectedId(rows[0].id);
    }
  }, [rows, selectedId]);

  const selected = rows.find((r) => r.id === selectedId) ?? null;

  return (
    <div className="workspace">
      <nav className="rail">
        <div className="section">
          <div className="eyebrow">Mine</div>
          <RailItem
            label="Needs attention"
            count={snap.mine.length}
            state="blocked"
            selected={selection.side === "mine" && selection.bucket === "all"}
            onClick={() => setSelection({ side: "mine", bucket: "all" })}
          />
          {MINE_RAIL.map((b) => {
            const n = snap.mine.filter((p) => p.bucket === b).length;
            if (n === 0 && b === "draft") return null;
            return (
              <RailItem
                key={b}
                label={MINE_BUCKET_LABEL[b]}
                count={n}
                state={b}
                selected={selection.side === "mine" && selection.bucket === b}
                onClick={() => setSelection({ side: "mine", bucket: b })}
              />
            );
          })}
          <RailItem
            label="Recently merged"
            count={snap.merged.length}
            state="none"
            selected={selection.side === "merged"}
            onClick={() => setSelection({ side: "merged" })}
          />
        </div>

        <div className="section">
          <div className="eyebrow">To review</div>
          <RailItem
            label="Everything"
            count={queue.length}
            state="requested"
            selected={selection.side === "queue" && selection.bucket === "all"}
            onClick={() => setSelection({ side: "queue", bucket: "all" })}
          />
          {QUEUE_RAIL.map((b) => (
            <RailItem
              key={b}
              label={QUEUE_BUCKET_LABEL[b]}
              count={queue.filter((p) => p.bucket === b).length}
              state={b}
              selected={selection.side === "queue" && selection.bucket === b}
              onClick={() => setSelection({ side: "queue", bucket: b })}
            />
          ))}
        </div>

        <div className="rail-foot">
          Watching <b>{snap.org}</b> for <span className="mono">Team Review - READY</span>.
          Read-only — actions happen on GitHub.
        </div>
      </nav>

      <section className="center">
        <Header
          snap={snap}
          selection={selection}
          shown={rows.length}
          sort={sort}
          onToggleSort={() => setSort((s) => (s === "urgency" ? "oldest" : "urgency"))}
        />
        <div className="scroll" style={{ flex: 1, minHeight: 0 }}>
          {selection.side === "merged" ? (
            <MergedList snap={snap} />
          ) : selection.side === "queue" ? (
            <QueueList
              items={rows as QueuePr[]}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          ) : (
            <MineList
              items={rows as MinePr[]}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          )}
        </div>
      </section>

      <aside className="detail-pane">
        {selected ? <Detail pr={selected} /> : <NoSelection />}
      </aside>
    </div>
  );
}

function RailItem({
  label,
  count,
  state,
  selected,
  onClick,
}: {
  label: string;
  count: number;
  state: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button className="rail-item" aria-selected={selected} onClick={onClick}>
      <Dot state={state} />
      <span className="label">{label}</span>
      <span className="n">{count}</span>
    </button>
  );
}

function Header({
  snap,
  selection,
  shown,
  sort,
  onToggleSort,
}: {
  snap: Snapshot;
  selection: Selection;
  shown: number;
  sort: Sort;
  onToggleSort: () => void;
}) {
  const title =
    selection.side === "merged"
      ? "Recently merged"
      : selection.side === "mine"
        ? selection.bucket === "all"
          ? "Needs attention"
          : MINE_BUCKET_LABEL[selection.bucket]
        : selection.bucket === "all"
          ? "Review queue"
          : QUEUE_BUCKET_LABEL[selection.bucket];

  const sub =
    selection.side === "mine"
      ? `${snap.mineCounts.blocked} blocked · ${snap.mineCounts.ready} ready`
      : selection.side === "queue"
        ? `${shown} need a human · ${snap.hidden.alreadyApproved + snap.hidden.alreadyReviewed} hidden`
        : `last ${shown}`;

  return (
    <div className="center-head">
      <h2>{title}</h2>
      <span className="sub">{sub}</span>
      <div className="filters">
        <button
          className="chip toggle"
          onClick={onToggleSort}
          title={
            sort === "urgency"
              ? "Sorted by urgency. Click to sort oldest first."
              : "Sorted oldest first. Click to sort by urgency."
          }
        >
          Sort: {sort}
        </button>
      </div>
    </div>
  );
}

function MineList({
  items,
  selectedId,
  onSelect,
}: {
  items: MinePr[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (items.length === 0) {
    return <Empty glyph="◎">Nothing here. Enjoy it.</Empty>;
  }
  return (
    <>
      {items.map((pr) => (
        <button
          key={pr.id}
          className="pr-row"
          aria-selected={pr.id === selectedId}
          onClick={() => onSelect(pr.id)}
          onDoubleClick={() => open(pr.url)}
        >
          <span className={`accent dot-${pr.bucket}`} />
          <div className="main">
            <div className="headline">
              <span className="slug">{pr.slug}</span>
              <span className="title">{pr.title}</span>
              {pr.isDraft && <span className="chip plain">draft</span>}
            </div>
            <div className="facts">
              <CiBadge ci={pr.ci} lg />
              <ReviewLabel review={pr.review} text={pr.reviewText} lg />
              {pr.approvedBy.length > 0 && (
                <span className="s-none">{approvedByLabel(pr.approvedBy)}</span>
              )}
              {pr.conflicting && <span className="s-fail">merge conflicts</span>}
            </div>
          </div>
          <div className="right">
            <span className="age">{pr.age}</span>
            <span
              className="open"
              onClick={(e) => {
                e.stopPropagation();
                open(pr.url);
              }}
            >
              open ↗
            </span>
          </div>
        </button>
      ))}
    </>
  );
}

function QueueList({
  items,
  selectedId,
  onSelect,
}: {
  items: QueuePr[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  if (items.length === 0) {
    return <Empty glyph="✓">The queue is clear.</Empty>;
  }

  return (
    <>
      {items.map((pr) => (
        <div key={pr.id}>
          <button
            className="pr-row"
            aria-selected={pr.id === selectedId}
            onClick={() => onSelect(pr.id)}
            onDoubleClick={() => open(pr.url)}
          >
            <span className={`accent dot-${pr.bucket}`} />
            <div className="main">
              <div className="headline">
                <span className="slug">{pr.slug}</span>
                <span className="title">{pr.title}</span>
                {pr.isDraft && <span className="chip plain">draft ⚠</span>}
              </div>
              <div className="facts">
                <span className="s-none">{pr.author}</span>
                <CiBadge ci={pr.ci} lg />
                {pr.approvals.length > 0 && (
                  <span className="s-approved_needs_more">
                    {approvedByLabel(pr.approvals)}
                  </span>
                )}
                {pr.conflicting && <span className="s-fail">merge conflicts</span>}
              </div>
            </div>
            <div className="right">
              <span className="age">
                {pr.age}
              </span>
              <span
                className="open"
                onClick={(e) => {
                  e.stopPropagation();
                  open(pr.url);
                }}
              >
                open ↗
              </span>
            </div>
          </button>
          {pr.siblingNote && <div className="sibling-note">{pr.siblingNote}</div>}
        </div>
      ))}
    </>
  );
}

function MergedList({ snap }: { snap: Snapshot }) {
  if (snap.merged.length === 0) {
    return <Empty glyph="◌">Nothing merged recently.</Empty>;
  }
  return (
    <>
      {snap.merged.map((pr) => (
        <button key={pr.url} className="pr-row" onClick={() => open(pr.url)}>
          <span className="accent dot-approved" />
          <div className="main">
            <div className="headline">
              <span className="slug">{pr.slug}</span>
              <span className="title">{pr.title}</span>
            </div>
          </div>
          <div className="right">
            <span className="age">{pr.age}</span>
            <span className="open">{pr.state} ↗</span>
          </div>
        </button>
      ))}
    </>
  );
}

function NoSelection() {
  return (
    <div className="detail-head">
      <div className="slug">—</div>
      <div className="title" style={{ color: "var(--text-faint)", fontWeight: 400, fontSize: 13 }}>
        Select a pull request to see its checks and history.
      </div>
    </div>
  );
}

function isMine(pr: MinePr | QueuePr): pr is MinePr {
  return "review" in pr;
}

function Detail({ pr }: { pr: MinePr | QueuePr }) {
  const ciChip =
    pr.ci.state === "fail"
      ? "solid-red"
      : pr.ci.state === "pass"
        ? "solid-green"
        : pr.ci.state === "pending"
          ? "solid-amber"
          : "plain";

  const ciWord =
    pr.ci.state === "fail"
      ? "CI failing"
      : pr.ci.state === "pass"
        ? "CI green"
        : pr.ci.state === "pending"
          ? "CI running"
          : "No checks";

  const checks: Check[] = pr.checks;
  const timeline: TimelineEntry[] = pr.timeline;

  return (
    <>
      <div className="detail-head">
        <div className="slug">{pr.slug}</div>
        <div className="title">{pr.title}</div>
        <div className="chips">
          <span className={`chip ${ciChip}`}>{ciWord}</span>
          <span className="chip plain">{pr.age} old</span>
          {isMine(pr) ? (
            <span className={`chip plain s-${pr.review}`}>{pr.reviewText}</span>
          ) : (
            <span className="chip plain">{pr.author}</span>
          )}
          {pr.conflicting && <span className="chip solid-red">conflicts</span>}
        </div>
      </div>

      <div className="scroll" style={{ flex: 1, minHeight: 0 }}>
        <div className="detail-section">
          <div className="eyebrow">
            Checks {checks.length > 0 && <span style={{ opacity: 0.7 }}>· {checks.length}</span>}
          </div>
          {checks.length === 0 && (
            <div style={{ fontSize: 12, color: "var(--text-faint)" }}>
              No checks reported on the head commit.
            </div>
          )}
          {/* Matrix jobs report several check runs under one name, so the name
              alone is not a stable key. */}
          {checks.map((c, i) => (
            <div className="check-row" key={`${c.name}-${i}`}>
              <Dot state={c.state} lg />
              <span className="name" title={c.name}>
                {c.name}
              </span>
              <span className={`state s-${c.state}`}>{c.state}</span>
              <span className="dur">{c.duration || "—"}</span>
            </div>
          ))}
        </div>

        <div className="detail-section">
          <div className="eyebrow">Timeline</div>
          {timeline
            .slice()
            .reverse()
            .map((t, i) => (
              <div className="tl-row" key={`${t.at}-${i}`}>
                <span className="at">{t.age}</span>
                <span className="text">{t.text}</span>
              </div>
            ))}
        </div>

        {!isMine(pr) && pr.requested.length > 0 && (
          <div className="detail-section">
            <div className="eyebrow">Reviewers requested</div>
            <div style={{ fontSize: 12, color: "var(--text-soft)", lineHeight: 1.6 }}>
              {pr.requested.join(", ")}
            </div>
          </div>
        )}
      </div>

      <div className="detail-foot">
        <button className="btn-primary" onClick={() => open(pr.url)}>
          Open on GitHub ↗
        </button>
      </div>
    </>
  );
}
