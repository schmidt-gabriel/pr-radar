# Architecture

How PR Radar is put together, and why the awkward parts are the way they are.

## The shape of it

```
GitHub GraphQL
      │
      │  3 searches per poll (mine, review queue, recently merged)
      ▼
  github.rs ── raw types, token discovery
      │
      ▼
  derive.rs ── ALL the rules live here
      │           buckets · CI rollup · approval semantics · ordering · events
      ▼
  model.rs  ── the snapshot the UI receives (semantic only, no colours)
      │
      ▼
  poller.rs ── 60s loop, notification bookkeeping
      │
      ▼
   lib.rs   ── tray, windows, shortcuts, commands
      │
      │  one `feed` event, pushed to every window
      ▼
  React ──── Popover · Triage · Timeline
```

Two rules keep this honest:

**All derivation happens once, in `derive.rs`.** The three views never
recompute anything. That is what makes it impossible for the popover to claim
two blocked PRs while the timeline claims three.

**No colours cross the IPC bridge.** The backend sends states (`fail`,
`blocked`, `no_approval`); the frontend maps them to colours in one place,
`styles.css`. The design doc put colours in the data, which would have meant
three copies of the palette drifting apart.

## Talking to GitHub

The two skills this replaces (`my-prs`, `prs-to-review`) shell out to `gh` once
per pull request. That is fine for a one-off report and wrong for a poller:
thirty labelled PRs meant thirty subprocesses and thirty round trips, every
minute.

One GraphQL search returns the same fields for a hundred PRs, so a poll is three
requests total, run concurrently:

| Query | Purpose |
|---|---|
| `is:pr is:open author:@me` | your open PRs |
| `is:pr is:open org:<org> label:"<label>"` | the org-wide review queue |
| `is:pr author:@me is:merged sort:updated-desc` | recent context |

Search returns a union type, so non-PR nodes arrive as `{}`. Those are skipped
rather than failing the whole poll.

### Finding a token

No token is stored. `Github::discover` reads `GH_TOKEN` or `GITHUB_TOKEN`, and
otherwise asks the GitHub CLI.

Locating `gh` is harder than it looks. An app launched from Finder or a desktop
launcher inherits a minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), not your
shell's, so a Homebrew, Snap or `~/.local` install is invisible to it. The
lookup therefore tries `PATH`, then a list of absolute install prefixes, then
falls back to asking your login shell.

It also distinguishes "`gh` is missing" from "`gh` refused", because those need
opposite advice. Telling somebody who is already logged in to run
`gh auth login` sends them down a dead end.

## The rules

### Your pull requests

| Bucket | Meaning |
|---|---|
| Blocked | changes requested, or CI failing |
| Ready | approved, green, mergeable |
| Waiting | everything else, including approved-but-not-enough |
| Draft | `isDraft` |

### The review queue

Start from every PR carrying the review label, then drop:

- anything you authored (GitHub search has no negative-author filter, so this is
  a client-side pass)
- anything you already reviewed, in any state, including a bare comment
- anything with enough approval: an explicit `APPROVED` decision, or no decision
  at all plus a real human approval

A PR still marked `REVIEW_REQUIRED` despite an approval has not cleared branch
protection, so it stays. The counts of what was dropped are reported, so a short
list never reads as an empty queue.

Survivors are ordered: asked of you (oldest first, they have waited on *you*),
then no approval (newest first, to catch PRs before they go stale), then partial
approval.

### Traps worth knowing

These are the things that look right and are wrong. Each has a test in
`derive.rs`.

**A check run reports `conclusion: ""` while it is still running.** Empty
string, not null. So the obvious `conclusion // status` fallback picks the wrong
branch and reports a running check as passed.

**A trailing `COMMENTED` review must not erase a standing approval.** Reading
only the newest review per author does exactly that. Reviews are collapsed to
each author's latest *verdict*, where a comment is not a verdict.

**A `DISMISSED` review must revoke the approval it followed.** It is a verdict,
and a later one wins.

**Bot reviews never count as approval.** CodeRabbit and Copilot comment at
length but do not satisfy branch protection. Counting them would silently empty
the review queue, which is the worst possible failure here: it looks like good
news.

**Repeated actions collapse.** Nine review comments posted in one sitting are
one action. Both the timeline feed and the per-PR history fold runs of the same
event into a single row with a `×N`. The per-PR version keeps the run's
*newest* timestamp, since entries there are built oldest-first and would
otherwise report the wrong time.

## The timeline

Every event carries a real GitHub timestamp: PR openings, review submissions,
and check-run completions. Nothing is stamped with poll time, which is why the
feed still reads correctly after the app has been closed for a week.

One CI event is emitted per PR rather than one per check, because forty green
rows per PR would bury everything else.

The single exception is "review requested from you", which GitHub does not
timestamp. It is anchored to the PR's last update, the closest honest proxy.

## Notifications

Fired for CI failures, approvals and change requests on your PRs, and review
requests aimed at you or one of your teams.

Event IDs are persisted to `seen-events.json`, so relaunching never
re-announces yesterday's failure. On the very first launch, with no file
present, the store is seeded silently rather than firing a notification for
every open PR at once.

## The platform layer

Everything above is shared. The tray is where macOS and Linux genuinely
disagree, so all three channels are used for the counts.

| | macOS | Linux |
|---|---|---|
| Count beside the icon (`set_title`) | yes | usually, panel-dependent |
| Tooltip (`set_tooltip`) | yes | **not supported** |
| Status line in the menu | yes | yes |
| Left-click opens the popover | yes | **no click events at all** |
| Icon geometry for anchoring | yes | **none reported** |

Under AppIndicator the tray reports neither clicks nor geometry, so the menu is
the primary interaction on Linux and carries a **Show popover** entry. Placement
falls back to the top-right corner. When a tray *does* sit in the lower half of
the screen, the popover flips to open upward instead of off the bottom edge.

The icon differs by necessity: macOS gets a template image, meaning black shapes
plus alpha that the system recolours per appearance. Linux has no such concept,
so the same file would be an invisible black silhouette on a dark panel. It gets
a light-coloured icon instead.

Two more platform traps:

**Binding `metaKey` for shortcuts is the classic macOS-first bug.** Off macOS,
Meta is the Super key, so every shortcut silently does nothing. `platform.ts`
picks the modifier and the UI renders the chord the way the host writes it.

**Tauri's Linux webview reports `AppleWebKit` in its user agent.** Platform
detection matches on `Macintosh`, because a looser test reads Linux as macOS.

Windows is not supported. The blockers are `gh` discovery, which looks for `gh`
rather than `gh.exe` and shells out to `$SHELL -lc`, and installer targets.
