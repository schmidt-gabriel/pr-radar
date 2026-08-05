/**
 * Platform detection for the webview.
 *
 * The Rust side knows its target at compile time, but the frontend is one
 * bundle running on all three, so it has to ask. This is read once at startup
 * and stamped onto the document element for CSS to key off.
 */

export type Platform = "macos" | "windows" | "linux";

export function detectPlatform(): Platform {
  const ua = typeof navigator === "undefined" ? "" : navigator.userAgent;
  if (/Macintosh|Mac OS X/.test(ua)) return "macos";
  if (/Windows/.test(ua)) return "windows";
  return "linux";
}

export const PLATFORM: Platform = detectPlatform();
export const IS_MAC = PLATFORM === "macos";

/**
 * The primary chord modifier. Binding `metaKey` unconditionally is the classic
 * macOS-first bug: on Linux and Windows the Meta key is Super, so every
 * shortcut silently does nothing.
 */
export function hasMod(e: KeyboardEvent): boolean {
  return IS_MAC ? e.metaKey : e.ctrlKey;
}

/** Render a shortcut the way the host platform writes it. */
export function chord(key: string): string {
  return IS_MAC ? `⌘${key}` : `Ctrl+${key}`;
}

/** The global show/hide shortcut, which differs to avoid host conflicts. */
export const GLOBAL_TOGGLE = IS_MAC ? "⌘⇧P" : "Ctrl+Alt+P";
