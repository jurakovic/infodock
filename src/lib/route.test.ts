import { describe, expect, it } from "vitest";
import { settingsPage } from "./route";

describe("settingsPage", () => {
  it("reads the page from the path", () => {
    expect(settingsPage("/settings/bar")).toBe("bar");
    expect(settingsPage("/settings")).toBe("general");
    expect(settingsPage("/bar")).toBe("general");
  });
});
