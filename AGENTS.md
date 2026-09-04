# AGENTS.md

Context for AI agents working on this repo. Human docs:
[README.md](README.md) (what it is), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
(how it works), [docs/DEVELOPING.md](docs/DEVELOPING.md) (how to build it).

PR Radar is a Tauri 2 + React tray app for macOS and Linux. It polls GitHub
every 60s and shows your pull requests, their CI, and your review queue across
three surfaces: a tray popover, a triage window, and a timeline feed. It is
strictly read-only; every row opens GitHub.

Requires Rust 1.77.2+, Node 22+, and the GitHub CLI logged in.

## Commands

```bash
npm run app        # dev build with frontend hot reload
npm run app:build  # bundle installers
npm run build      # frontend only; runs tsc, so this is the type check
npm test           # cargo test (10 tests, all in derive.rs)
cd src-tauri && cargo run --example snapshot            # readable dump from live GitHub
cd src-tauri && cargo run --example snapshot -- --json  # the exact UI payload
```

CI runs on every push and will fail the build on any of these, so run them
before committing:

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
npm run build
```

## Invariants

Break these and the app becomes subtly inconsistent rather than obviously
broken, which is the worst failure mode here.

1. **All derivation happens in `src-tauri/src/derive.rs`.** Buckets, CI rollup,
   approval semantics, ordering and events are decided once. The three views
   never recompute anything. This is what makes it impossible for the popover
   to report two blocked PRs while the timeline reports three.
2. **No colours cross the IPC bridge.** The backend sends semantic states
   (`fail`, `blocked`, `no_approval`); `src/styles.css` is the only place a
   colour is chosen. Do not add colour fields to `model.rs`.
3. **Read-only.** No mutating GitHub calls. Actions belong on GitHub.

## Traps

Each of these looks correct and is wrong. Most have a regression test in
`derive.rs`; keep them passing.

- **A check run reports `conclusion: ""` while running.** Empty string, not
  null, so `conclusion // status` picks the wrong branch and calls a running
  check passed.
- **A trailing `COMMENTED` review must not erase a standing approval.** Reading
  only the newest review per author does exactly that. Collapse to each
  author's latest *verdict*; a comment is not a verdict.
- **A `DISMISSED` review must revoke the approval it followed.** It is a
  verdict, and the later one wins.
- **Bot reviews never count as approval.** CodeRabbit and Copilot comment but
  do not satisfy branch protection. Counting them silently empties the review
  queue, which looks like good news.
- **`gh` is not on PATH for a GUI-launched app.** Finder and desktop launchers
  hand over a minimal PATH, so a Homebrew, Snap or `~/.local` install is
  invisible. `github.rs` searches absolute prefixes and falls back to the login
  shell. Keep "gh is missing" and "gh refused" as distinct errors; they need
  opposite advice.
- **Binding `metaKey` for shortcuts breaks every non-Mac platform,** where Meta
  is the Super key. Use `hasMod()` from `src/lib/platform.ts`.
- **Tauri's Linux webview reports `AppleWebKit` in its user agent.** Platform
  detection matches on `Macintosh`. A looser test reads Linux as macOS.
- **`trafficLightPosition.y` is not a top inset.** tao resizes the title bar
  container to `buttonHeight + y` and the buttons keep their offset inside it,
  so they end up centred near `y`. The `.titlebar` height in `styles.css` is
  tuned to match and must move whenever `y` does, or the buttons drift out of
  line with the window content.
- **Everything in `public/` ships inside the bundle.** The dev fixture lives in
  `dev/` and is served by a dev-only Vite middleware for exactly this reason:
  it is a dump of real PR data.
- **The Vite port is pinned with `strictPort`** (5179). Tauri's `devUrl` is a
  fixed address, so a silent fallback to the next free port loads whatever else
  is on 5173 into the app window. Refusing to start is the correct failure.
- **`#[cfg]`-disabled code is never type-checked.** Linux tray paths do not
  compile on a Mac. Keep platform branches as small as possible (ideally just a
  constant) and let the CI Linux job be the real check.
- **Release assets are named `PR.Radar_*`, not `pr-radar_*`.** GitHub
  substitutes the space in the product name.

## Layout

```
src-tauri/src/
  github.rs   GraphQL client and token discovery
  derive.rs   Every rule. The only place state is decided
  model.rs    The payload the UI receives, semantic only
  poller.rs   60s loop and notification bookkeeping
  lib.rs      Tray, windows, shortcuts, commands
src/
  lib/        Feed subscription, formatting, platform detection
  views/      Popover, Triage, Timeline
  styles.css  Design tokens, and the only place colours are decided
```

## Conventions

- **Everything in the repo is English**: code, comments, commit messages, docs.
  This holds even when the request came in Portuguese.
- Comments explain *why*, not what. The valuable ones here record a trap that
  was hit, so future readers do not undo the fix.
- Icons are generated, not hand-drawn. Edit the parameters at the top of
  `src-tauri/icons/make_*.py` and rerun; do not hand-edit the PNGs.
- Commit messages: a short subject, then what changed and the reasoning behind
  a non-obvious choice.

## Verifying changes

Do not assume; this codebase has a habit of punishing that.

- **Derivation logic**: `cargo run --example snapshot` against live GitHub. It
  prints the real buckets, queue and events in a few lines.
- **UI**: dump a fixture to `dev/snapshot.json`, then `npm run dev`. Outside
  the Tauri webview the views load that fixture, so a plain browser renders
  real data with no Rust build. `?window=popover` renders the popover.
- **Anything native** (tray, Dock behaviour, transparent window corners,
  traffic lights): cannot be verified from the DOM or from CSS arithmetic. Ask
  the user for a screenshot rather than reporting it as confirmed.

## Known gaps

- **Windows is unsupported.** The blockers are `gh` discovery, which looks for
  `gh` rather than `gh.exe` and shells out to `$SHELL -lc`, and installer
  targets.
- **Linux is built and checked in CI but has never run on a real desktop.** The
  tray behaviour in particular is unverified.
- **Builds are unsigned.** macOS Gatekeeper blocks the first launch until the
  quarantine attribute is cleared.
- The default `org` and `label` in `poller.rs` point at the author's employer.
  They are config defaults, not an assumption baked into the logic; the app
  works against any org and any label.
