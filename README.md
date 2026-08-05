<img width="256" height="256" alt="icon" src="https://github.com/user-attachments/assets/8425b4a5-2eb2-4f7b-8b36-b108c606f03d" />


# PR Radar

A macOS menu-bar app for the status of your PRs, their CI, and your review queue.
Read-only: every row opens GitHub.

Implemented from the `PR Radar.dc.html` design doc, which explored three
directions and recommended Tauri + React. All three ship here:

| | Surface | Where |
|---|---|---|
| **1a** | Menu-bar popover — the 10×/day glance | tray icon, or ⌘⇧P |
| **1b** | Desktop triage window — list + detail | main window, ⌘1 |
| **1c** | Timeline — "when did each thing happen" | main window, ⌘2 |

All three read the same derived snapshot from one Rust polling module, so they
cannot disagree with each other.

## Running it

Requires the [GitHub CLI](https://cli.github.com) logged in (`gh auth login`) —
the app reads its token, or `GH_TOKEN`/`GITHUB_TOKEN` if set. Needs the `repo`
and `read:org` scopes.

```bash
npm install
npm run app
```

To build a distributable `.app`:

```bash
npm run app:build
```

## What it shows

Two queries, encoding the rules from the `my-prs` and `prs-to-review` skills.

**Mine** — your open PRs, bucketed:

- **Blocked on you** — changes requested, or CI failing
- **Ready to merge** — approved, green, mergeable
- **Waiting on review** — including "approved but branch protection wants another"
- **Drafts**

**To review** — org-wide PRs labeled `Team Review - READY`, authored by someone
else, minus the ones that no longer need you:

- already reviewed by you (any state, including a bare comment)
- already approved — an explicit `APPROVED` decision, or no decision plus a real
  human approval. A PR still marked `REVIEW_REQUIRED` despite an approval has
  not cleared branch protection, so it stays.

The count of what was hidden is always shown, so a short list never reads as an
empty queue.

Survivors are ordered: asked of you (oldest first) → no approval (newest first)
→ partial approval (oldest first). When a sibling PR on the same ticket was
filtered out for being approved, the row says so, so you know you are reviewing
half of a cross-repo change.

**Timeline** — reviews, CI results and PR openings from the last 14 days. Every
event carries a real GitHub timestamp, so the feed still reads correctly after
the app has been closed for a week. Bursts of the same action on the same PR
collapse into one row with a `×N`.

## Notifications

Fired for CI failures, approvals and change requests on your PRs, and for review
requests aimed at you or one of your teams. Event ids are persisted to
`~/Library/Application Support/dev.schmidt.pr-radar/seen-events.json`, so
relaunching does not re-announce yesterday's failure. The first ever launch
seeds silently instead of firing a notification per open PR.

## Configuration

`~/Library/Application Support/dev.schmidt.pr-radar/config.json`:

```json
{ "org": "edsights", "label": "Team Review - READY", "pollSeconds": 60, "notify": true }
```

The view toggles (relative vs absolute dates, hiding partially-approved PRs)
live in the window titlebar and persist locally.

## Layout

```
src-tauri/src/
  github.rs   GraphQL client. One search per query, not one request per PR —
              a 30-PR queue was 30 subprocesses under `gh`.
  derive.rs   Every rule from both skills. The only place buckets, CI rollup,
              approval semantics and queue ordering are decided.
  model.rs    The payload the UI receives. Semantic only — no colors cross the
              bridge; those live in the frontend tokens.
  poller.rs   The 60s loop and notification bookkeeping.
  lib.rs      Tray, windows, shortcuts, commands.
src/
  views/      Popover (1a), Triage (1b), Timeline (1c)
  styles.css  Design tokens lifted from the design doc
```

### Working on the UI without a Rust build

```bash
cd src-tauri && cargo run --example snapshot -- --json > ../dev/snapshot.json
npm run dev
```

Outside the Tauri webview the views load that fixture, so the browser renders
real data. `?window=popover` renders the popover instead of the triage shell.

The fixture is served by a dev-only Vite middleware and deliberately does not
live in `public/`, since everything there is copied into the production bundle
and a dump of real PR data has no business shipping inside the app.

Both icons are generated rather than hand-drawn, from pure-Python rasterizers
with no image-library dependency:

```bash
cd src-tauri/icons
python3 make_tray_icon.py .   # menu-bar template image
python3 make_app_icon.py .    # app icon: PNG sizes, .icns, .ico
```

`make_tray_icon.py` writes a black-on-alpha template PNG at 36px, an exact 2x of
the 18pt height `tray-icon` scales everything to. `make_app_icon.py` renders the
Big Sur squircle and hands the set to `iconutil`; sizes at or below 64px get
deliberately simplified geometry (fewer rings, heavier strokes, one contact)
because the detailed artwork turns to mush when downscaled that far.

`dev/iconprev.html`, served by `npm run dev`, shows both at every size against a
checkerboard.

`cargo run --example snapshot` (without `--json`) prints a readable summary —
the fastest way to check the derivation rules against live GitHub.

## Notes

- The Vite port is pinned with `strictPort`. Tauri's `devUrl` is a fixed
  address, so a silent fallback to the next free port loads whatever else is on
  5173 into the app window.
- Bot reviews (`coderabbitai`, `copilot-pull-request-reviewer`, anything
  `[bot]`) never count as approval — treating them as approvals would quietly
  empty the review queue.
- A check run reports `conclusion: ""` — not null — while it is still running,
  so a plain `conclusion // status` fallback picks the wrong branch and reports
  a running check as passed.
- A trailing `COMMENTED` review must not erase a standing approval, and a
  `DISMISSED` one must revoke it. Both are covered by tests in `derive.rs`.
