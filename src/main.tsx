import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans/700.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./styles.css";

import App from "./App";
import Popover from "./views/Popover";
import { PLATFORM } from "./lib/platform";

// Lets CSS key off the host, e.g. only macOS needs room for traffic lights.
document.documentElement.dataset.platform = PLATFORM;

/**
 * Both windows load the same bundle; the label decides which surface renders.
 * Outside the Tauri webview there are no window internals to read, so fall back
 * to the triage shell instead of throwing before React ever mounts.
 */
function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return new URLSearchParams(location.search).get("window") ?? "main";
  }
}

const isPopover = windowLabel() === "popover";

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isPopover ? <Popover /> : <App />}</StrictMode>,
);
