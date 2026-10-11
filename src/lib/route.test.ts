import { describe, expect, it } from "vitest";
import { resolveApp, settingsPage } from "./route";

describe("resolveApp", () => {
  it("prefers the Tauri window label", () => {
    expect(resolveApp("settings", "/bar")).toBe("settings");
    expect(resolveApp("hint", "/")).toBe("hint");
  });
  it("falls back to the path in a plain browser", () => {
    expect(resolveApp(null, "/popup")).toBe("popup");
    expect(resolveApp(null, "/settings/bar")).toBe("settings");
  });
  it("defaults to the bar", () => {
    expect(resolveApp(null, "/")).toBe("bar");
    expect(resolveApp("unknown", "/nope")).toBe("bar");
  });
});

describe("settingsPage", () => {
  it("returns the page segment or general", () => {
    expect(settingsPage("/settings/bar")).toBe("bar");
    expect(settingsPage("/settings")).toBe("general");
    expect(settingsPage("/bar")).toBe("general");
  });
});
