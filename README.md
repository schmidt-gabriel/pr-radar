
<p align="center">
  <img width="256" height="256" alt="icon" src="https://github.com/user-attachments/assets/8425b4a5-2eb2-4f7b-8b36-b108c606f03d" />
</p>

# PR Radar

A tray app for the status of your PRs, their CI, and your review queue.
Read-only: every row opens GitHub. Runs on macOS and Linux.

Implemented from the `PR Radar.dc.html` design doc, which explored three
directions and recommended Tauri + React. All three ship here:

| | Surface | Where |
|---|---|---|
| **1a** | Tray popover — the 10×/day glance | tray icon, or the global shortcut |
| **1b** | Desktop triage window — list + detail | main window, `⌘1` / `Ctrl+1` |
| **1c** | Timeline — "when did each thing happen" | main window, `⌘2` / `Ctrl+2` |

All three read the same derived snapshot from one Rust polling module, so they
cannot disagree with each other.

| Shortcut | macOS | Linux |
|---|---|---|
| Show/hide the popover (global) | `⌘⇧P` | `Ctrl+Alt+P` |
| Refresh | `⌘R` | `Ctrl+R` |
| Triage / Timeline | `⌘1` / `⌘2` | `Ctrl+1` / `Ctrl+2` |

`Ctrl+Alt+P` differs from the macOS chord because desktop environments reserve
the Super key heavily.

## Install

Grab the latest build from [**Releases**](https://github.com/schmidt-gabriel/pr-radar/releases).
Every release is built by CI for both platforms.

**macOS** — take `PR Radar_<version>_universal.dmg`. It covers both Apple
silicon and Intel. The app is unsigned, so Gatekeeper will refuse it on first
launch. Either right-click the app and choose **Open**, or clear the quarantine
flag:

```bash
xattr -dr com.apple.quarantine "/Applications/PR Radar.app"
```

**Linux** — take whichever suits your distro:

```bash
sudo dpkg -i pr-radar_*_amd64.deb      # Debian, Ubuntu
sudo rpm -i pr-radar-*.x86_64.rpm      # Fedora, RHEL
chmod +x pr-radar_*.AppImage && ./pr-radar_*.AppImage   # anything else
```

The `.deb` and `.rpm` pull in their own runtime dependencies. For the AppImage
you need `libwebkit2gtk-4.1-0` and `libayatana-appindicator3-1` present, plus a
notification daemon, which every mainstream desktop already runs.

### One-time setup

The app needs the [GitHub CLI](https://cli.github.com) logged in:

```bash
gh auth login
```

It reads `gh`'s token, or `GH_TOKEN`/`GITHUB_TOKEN` if either is set. The token
needs the `repo` and `read:org` scopes. If you launch from Finder or a desktop
launcher and it cannot find `gh`, see the note at the bottom about minimal
`PATH`.

## Building from source

```bash
npm install
npm run app        # run it
npm run app:build  # bundle it
```

Bundles land in `src-tauri/target/release/bundle/`: `.app` and `.dmg` on macOS,
`.deb`, `.rpm` and AppImage on Linux. Tauri filters the target list to whatever
the host can produce, so the same config works on both.

### Linux build dependencies

Tauri builds against the system WebKit, and the tray needs an AppIndicator
implementation. On Debian or Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libayatana-appindicator3-dev librsvg2-dev
```

Fedora uses `webkit2gtk4.1-devel`, `libappindicator-gtk3-devel` and
`librsvg2-devel`; Arch uses `webkit2gtk-4.1`, `libappindicator-gtk3` and
`librsvg`.

TLS is `rustls`, not `native-tls`, so there is no OpenSSL development package
to chase.

## Platform support

The polling, derivation, storage and UI are shared. What differs is the tray,
because the three platforms genuinely disagree about what a tray can do.

| | macOS | Linux |
|---|---|---|
| Count beside the tray icon | yes | usually, panel-dependent |
| Tray tooltip | yes | not supported |
| Status line in the tray menu | yes | yes |
| Left-click opens the popover | yes | no, use the menu |
| Popover anchored to the icon | yes | no icon geometry, so it anchors top-right |

Under AppIndicator the tray reports neither click events nor icon geometry, so
on Linux the menu is the primary interaction and carries a **Show popover**
entry. The status line in that menu exists because it is the one channel every
platform renders reliably.

The tray icon also differs by necessity. macOS gets a template image, meaning
black shapes plus alpha that the system recolors per appearance. Linux has no
such concept, so the same file would be a black silhouette on a dark panel; it
gets a light-colored icon instead.

Windows is not done. The blockers are `gh` discovery, which looks for `gh`
rather than `gh.exe`, and installer targets.

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
`seen-events.json` in the data directory below, so relaunching does not
re-announce yesterday's failure. The first ever launch seeds silently instead of
firing a notification per open PR.

## Configuration

The data directory is `~/Library/Application Support/dev.schmidt.pr-radar/` on
macOS and `~/.local/share/dev.schmidt.pr-radar/` on Linux (or
`$XDG_DATA_HOME/dev.schmidt.pr-radar/` when that is set).

`config.json` there:

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

`make_tray_icon.py` writes two files: `tray.png`, a black-on-alpha template at
36px, an exact 2x of the 18pt height `tray-icon` scales everything to on macOS;
and `tray-color.png` at 48px in near-white, for Linux panels where a template
image would be an invisible black silhouette.

`make_app_icon.py` renders the squircle and hands the set to `iconutil`; sizes
at or below 64px get deliberately simplified geometry (fewer rings, heavier
strokes, one contact) because the detailed artwork turns to mush when downscaled
that far. It needs macOS for the `.icns` step; the PNGs and `.ico` it writes are
portable.

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
- Binding `metaKey` for shortcuts is the classic macOS-first bug: elsewhere Meta
  is the Super key, so every shortcut silently does nothing. `lib/platform.ts`
  picks the modifier, and the UI renders the chord the host writes it.
- Tauri's Linux webview reports `AppleWebKit` in its user agent, so platform
  detection has to match on `Macintosh`, not on `AppleWebKit`.
- `gh` is resolved by absolute path as well as `PATH`. An app launched from
  Finder or a desktop launcher inherits a minimal `PATH`, so an install under
  `/opt/homebrew`, `/snap` or `~/.local` is invisible to it.
