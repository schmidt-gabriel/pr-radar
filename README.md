
<p align="center">
  <img width="256" height="256" alt="icon" src="https://github.com/user-attachments/assets/8425b4a5-2eb2-4f7b-8b36-b108c606f03d" />
</p>

<h1 align="center">PR Radar</h1>

<p align="center">
  <b>Every pull request that needs you, one glance away.</b><br>
  A quiet tray app for macOS and Linux.
</p>

---

You open GitHub to check on a pull request. Twenty minutes later you are reading
a diff you did not mean to open, and you still do not know whether CI went red
on the branch you shipped this morning.

PR Radar puts that answer in your menu bar. One number tells you whether
anything is on fire. Click it, and you see exactly what changed.

## What you get

**Your PRs, sorted by what you can do about them.** Not a list. Four buckets:
blocked on you, ready to merge, waiting on someone else, drafts. A PR is
"blocked" when CI broke or someone requested changes, so the top of the list is
always the thing to fix next.

**A review queue that respects your time.** It hides what you already reviewed
and what already has enough approvals, then tells you how many it hid so a short
list never looks like an empty one. Bot reviews never count as approval, so
CodeRabbit cannot quietly empty your queue.

**A feed that answers "when did that happen".** CI results, approvals, change
requests and new PRs, grouped by day with real timestamps. Close the app for a
week and the history still reads correctly when you come back.

**Notifications only for things that need you.** Your CI broke. Someone approved
you. Someone asked for your review. It remembers what it already told you, so
relaunching never re-announces yesterday's failure.

**Nothing you can break.** It is strictly read-only. Every row opens GitHub,
where the actual decisions happen.

## Three ways to look

| | For | How |
|---|---|---|
| **Popover** | The ten-second check | Click the tray icon |
| **Triage window** | Sitting down to clear the queue | Full window, list and detail |
| **Timeline** | Catching up after time away | Day-by-day feed |

All three show the same picture, so they can never tell you different things.

## Install

Grab the latest build from [**Releases**](https://github.com/schmidt-gabriel/pr-radar/releases).

**macOS** — download `PR.Radar_<version>_universal.dmg`, which runs on both
Apple silicon and Intel. The app is unsigned, so the first launch needs a nudge:
right-click it and choose **Open**, or run

```bash
xattr -dr com.apple.quarantine "/Applications/PR Radar.app"
```

**Linux** — pick the one that suits you:

```bash
sudo dpkg -i PR.Radar_*_amd64.deb                       # Debian, Ubuntu
sudo rpm -i PR.Radar-*.x86_64.rpm                       # Fedora, RHEL
chmod +x PR.Radar_*.AppImage && ./PR.Radar_*.AppImage   # anything else
```

### Sign in

PR Radar borrows the login from the [GitHub CLI](https://cli.github.com), so
there is no password to type and no token to paste:

```bash
gh auth login
```

That is the whole setup. It starts watching immediately, and refreshes every
minute.

## Shortcuts

| | macOS | Linux |
|---|---|---|
| Show or hide the popover | `⌘⇧P` | `Ctrl+Alt+P` |
| Refresh now | `⌘R` | `Ctrl+R` |
| Triage / Timeline | `⌘1` / `⌘2` | `Ctrl+1` / `Ctrl+2` |

## Make it yours

By default it watches your own pull requests plus your team's review queue. You
can point it at a different organization, change which label marks a PR ready
for review, or slow the refresh down. See
[Configuration](docs/DEVELOPING.md#configuration).

---

Built with Tauri and React. Curious how it works, or want to build it yourself?
See [docs/DEVELOPING.md](docs/DEVELOPING.md).
