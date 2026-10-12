/** The page inside the Settings window, e.g. "/settings/bar" → "bar". */
export function settingsPage(pathname: string): string {
  const parts = pathname.split("/").filter((s) => s.length > 0);
  return parts[0] === "settings" && parts[1] ? parts[1] : "general";
}
