import type {
  Ci,
  EventKind,
  MineBucket,
  QueueBucket,
  ReviewState,
} from "../lib/types";

export const MINE_BUCKET_LABEL: Record<MineBucket, string> = {
  blocked: "Blocked on you",
  ready: "Ready to merge",
  waiting: "Waiting on review",
  draft: "Drafts",
};

export const QUEUE_BUCKET_LABEL: Record<QueueBucket, string> = {
  requested: "Asked of you",
  no_approval: "No approval",
  partial: "Partial approval",
};

/** Compact form for the popover rows, where the bucket rides above the title. */
export const QUEUE_BUCKET_TAG: Record<QueueBucket, string> = {
  requested: "REQUESTED",
  no_approval: "NO APPROVAL",
  partial: "PARTIAL",
};

export const EVENT_GLYPH: Record<EventKind, string> = {
  ci_fail: "✕",
  ci_pass: "✓",
  ci_running: "↻",
  approved: "✓",
  changes_requested: "✎",
  commented: "◇",
  opened: "↑",
  review_requested: "◆",
};

/** The state word shown at the right edge of a timeline row. */
export function eventState(kind: EventKind, mine: boolean): string {
  switch (kind) {
    case "ci_fail":
    case "changes_requested":
      return mine ? "blocked" : "needs work";
    case "ci_running":
      return "running";
    case "approved":
      return mine ? "ready" : "approved";
    case "review_requested":
      return "to review";
    case "ci_pass":
      return "green";
    case "opened":
      return mine ? "mine" : "to review";
    default:
      return "";
  }
}

export function Dot({ state, lg }: { state: string; lg?: boolean }) {
  return <span className={`dot ${lg ? "lg" : ""} dot-${state}`} />;
}

export function CiBadge({ ci, lg }: { ci: Ci; lg?: boolean }) {
  return (
    <span className={`ci ${lg ? "lg" : ""} s-${ci.state}`} title={ciTooltip(ci)}>
      <Dot state={ci.state} lg={lg} />
      {ci.text}
    </span>
  );
}

function ciTooltip(ci: Ci): string {
  if (ci.total === 0) return "No checks reported";
  const parts = [`${ci.passed} passed`];
  if (ci.failed) parts.push(`${ci.failed} failed`);
  if (ci.pending) parts.push(`${ci.pending} running`);
  return `${parts.join(" · ")} of ${ci.total}`;
}

export function ReviewLabel({
  review,
  text,
  lg,
}: {
  review: ReviewState;
  text: string;
  lg?: boolean;
}) {
  return (
    <span className={`s-${review}`} style={{ fontSize: lg ? 12 : 11 }}>
      {text}
    </span>
  );
}

export function RefreshIcon({ spinning }: { spinning?: boolean }) {
  return (
    <svg
      className={spinning ? "spinning" : undefined}
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="currentColor"
      aria-hidden
    >
      <path d="M8 3V1L5 4l3 3V5a3 3 0 1 1-3 3H3a5 5 0 1 0 5-5Z" />
    </svg>
  );
}

export function Empty({ glyph, children }: { glyph: string; children: React.ReactNode }) {
  return (
    <div className="empty">
      <div className="big">{glyph}</div>
      {children}
    </div>
  );
}

export function ErrorBanner({ message }: { message: string }) {
  // "gh is missing" and "gh refused" need opposite advice: telling someone who
  // is already logged in to run `gh auth login` sends them down a dead end.
  const missing = /could not find the `gh`/i.test(message);
  const needsLogin = !missing && /gh auth|not logged/i.test(message);

  return (
    <div className="banner">
      <b>Could not reach GitHub</b>
      {message}
      {needsLogin && (
        <>
          <br />
          Run <code className="mono">gh auth login</code> in a terminal, then refresh.
        </>
      )}
      {missing && (
        <>
          <br />
          Launch it from a terminal with <code className="mono">npm run app</code>, or
          give the bundle a token:{" "}
          <code className="mono">launchctl setenv GH_TOKEN "$(gh auth token)"</code>
        </>
      )}
    </div>
  );
}
