// The triage window: one shell hosting 1b and 1c over the same snapshot.

import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useFeed, usePrefs } from "./lib/feed";
import { syncedAgo } from "./lib/format";
import Timeline from "./views/Timeline";
import Triage from "./views/Triage";
import { Empty, ErrorBanner, RefreshIcon } from "./ui/atoms";

type View = "triage" | "timeline";

export default function App() {
  const { feed, refresh } = useFeed();
  const { prefs, update } = usePrefs();
  const [view, setView] = useState<View>("triage");
  const [spinning, setSpinning] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(t);
  }, []);

  // The tray and the popover can both ask for a specific view.
  useEffect(() => {
    const un = listen<string>("goto-view", (e) => {
      if (e.payload === "triage" || e.payload === "timeline") setView(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey) return;
      if (e.key === "r") {
        e.preventDefault();
        doRefresh();
      } else if (e.key === "1") {
        e.preventDefault();
        setView("triage");
      } else if (e.key === "2") {
        e.preventDefault();
        setView("timeline");
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

  return (
    <div className="app">
      <header className="titlebar" data-tauri-drag-region>
        <span className="name" data-tauri-drag-region>
          PR Radar
        </span>

        <div className="segmented">
          <button aria-selected={view === "triage"} onClick={() => setView("triage")}>
            Triage
          </button>
          <button aria-selected={view === "timeline"} onClick={() => setView("timeline")}>
            Timeline
          </button>
        </div>

        <span className="grow" data-tauri-drag-region />

        <button
          className={`chip toggle ${prefs.relativeTime ? "on" : ""}`}
          onClick={() => update({ relativeTime: !prefs.relativeTime })}
          title="Show ages as relative durations or calendar dates"
        >
          {prefs.relativeTime ? "3d" : "Aug 3"}
        </button>
        <button
          className={`chip toggle ${prefs.hideApprovedInQueue ? "on" : ""}`}
          onClick={() => update({ hideApprovedInQueue: !prefs.hideApprovedInQueue })}
          title="Hide PRs that already have one approval"
        >
          Hide partial
        </button>

        <span className="synced">{snap ? syncedAgo(snap.fetchedAt, now) : "connecting…"}</span>
        <button className="icon-btn" onClick={doRefresh} title="Refresh (⌘R)">
          <RefreshIcon spinning={spinning || feed.status === "loading"} />
        </button>
      </header>

      {feed.status === "error" && <ErrorBanner message={feed.data} />}

      {!snap && feed.status !== "error" && (
        <Empty glyph="◌">
          <span className="breathing">Talking to GitHub…</span>
        </Empty>
      )}

      {snap &&
        (view === "triage" ? (
          <Triage snap={snap} prefs={prefs} />
        ) : (
          <Timeline snap={snap} prefs={prefs} />
        ))}
    </div>
  );
}
