/** Which app a webview renders. Windows share one bundle and are told apart by label (SPEC §4.1). */
export type AppKind = "bar" | "hint" | "popup" | "settings";

const KINDS: readonly AppKind[] = ["bar", "hint", "popup", "settings"];

/**
 * Picks the app for a window. Inside Tauri the window label decides; in a plain
 * browser (dev preview, screenshots) the first path segment does, defaulting to the bar.
 */
export function resolveApp(windowLabel: string | null, pathname: string): AppKind {
  if (windowLabel && (KINDS as readonly string[]).includes(windowLabel)) {
    return windowLabel as AppKind;
  }
  const first = pathname.split("/").find((s) => s.length > 0) ?? "";
  return (KINDS as readonly string[]).includes(first) ? (first as AppKind) : "bar";
}

/** The sub-route inside the settings window, e.g. "/settings/bar" → "bar". */
export function settingsPage(pathname: string): string {
  const parts = pathname.split("/").filter((s) => s.length > 0);
  return parts[0] === "settings" && parts[1] ? parts[1] : "general";
}
