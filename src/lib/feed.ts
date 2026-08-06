import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Config, Feed, Prefs } from "./types";
import { DEFAULT_PREFS } from "./types";

/**
 * The one subscription every view uses. The Rust poller owns the data and
 * pushes a new snapshot on each tick; windows opened later catch up via the
 * initial `get_feed`.
 */
/**
 * True only when the bundle is running inside the Tauri webview. In a plain
 * browser (`npm run dev`) there is no IPC, so the views fall back to a snapshot
 * dumped by `cargo run --example snapshot -- --json`. Design-only work then
 * needs no Rust build, and the fixture is real data rather than invented rows.
 */
const IN_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function useFeed() {
  const [feed, setFeed] = useState<Feed>({ status: "loading" });

  useEffect(() => {
    let alive = true;

    if (!IN_TAURI) {
      fetch("/dev-snapshot.json")
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error("no fixture"))))
        .then((data) => alive && setFeed({ status: "ready", data }))
        .catch(() =>
          alive &&
          setFeed({
            status: "error",
            data: "Running outside Tauri. Generate a fixture with: cd src-tauri && cargo run --example snapshot -- --json > ../dev/snapshot.json",
          }),
        );
      return;
    }

    invoke<Feed>("get_feed")
      .then((f) => alive && setFeed(f))
      .catch(() => {});

    const unlisten = listen<Feed>("feed", (e) => alive && setFeed(e.payload));

    return () => {
      alive = false;
      unlisten.then((f) => f());
    };
  }, []);

  const refresh = useCallback(() => {
    if (IN_TAURI) void invoke("refresh");
  }, []);

  return { feed, refresh };
}

export function useConfig() {
  const [config, setConfig] = useState<Config | null>(null);

  useEffect(() => {
    invoke<Config>("get_config").then(setConfig).catch(() => {});
  }, []);

  const save = useCallback((next: Config) => {
    setConfig(next);
    void invoke("set_config", { config: next });
  }, []);

  return { config, save };
}

/** View preferences live in the webview — they change nothing on the server. */
export function usePrefs() {
  const [prefs, setPrefs] = useState<Prefs>(() => {
    try {
      const raw = localStorage.getItem("pr-radar:prefs");
      return raw ? { ...DEFAULT_PREFS, ...JSON.parse(raw) } : DEFAULT_PREFS;
    } catch {
      return DEFAULT_PREFS;
    }
  });

  const update = useCallback((patch: Partial<Prefs>) => {
    setPrefs((prev) => {
      const next = { ...prev, ...patch };
      try {
        localStorage.setItem("pr-radar:prefs", JSON.stringify(next));
      } catch {
        /* private mode, non-fatal */
      }
      return next;
    });
  }, []);

  return { prefs, update };
}

/** Read-only app: every row leads to GitHub, in the real browser. */
export function open(url: string) {
  void openUrl(url);
}

export function hidePopover() {
  void invoke("hide_popover");
}

export function openMain(view?: "triage" | "timeline") {
  void invoke("open_main", { view: view ?? null });
}

/** Guarded: in a plain browser there is no app to quit. */
export function quitApp() {
  if (IN_TAURI) void invoke("quit");
}
