import { afterEach, describe, expect, it, vi } from "vitest";
import { availableLanguages, registerCatalog, setLanguage, t } from "./i18n";

afterEach(() => {
  setLanguage("en");
});

describe("t", () => {
  it("returns plain messages", () => {
    expect(t("tray.quit")).toBe("Quit Infodock");
  });

  it("selects plural forms by count", () => {
    expect(t("common.items", { count: 1 })).toBe("1 item");
    expect(t("common.items", { count: 0 })).toBe("0 items");
    expect(t("common.items", { count: 12 })).toBe("12 items");
  });

  it("leaves unknown placeholders alone", () => {
    registerCatalog("xx", { "app.name": "{who} dock" });
    setLanguage("xx");
    expect(t("app.name", { other: 1 })).toBe("{who} dock");
  });

  it("falls back to English per key and warns once", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    registerCatalog("de", { "app.name": "Infodock DE" });
    setLanguage("de");
    expect(t("app.name")).toBe("Infodock DE");
    expect(t("tray.lock")).toBe("Lock");
    expect(t("tray.lock")).toBe("Lock");
    expect(warn).toHaveBeenCalledTimes(1);
    warn.mockRestore();
  });

  it("uses the target language's plural rules", () => {
    registerCatalog("pl", { "common.items": { one: "{count} element", few: "{count} elementy", many: "{count} elementów", other: "{count} elementu" } });
    setLanguage("pl");
    expect(t("common.items", { count: 1 })).toBe("1 element");
    expect(t("common.items", { count: 3 })).toBe("3 elementy");
    expect(t("common.items", { count: 5 })).toBe("5 elementów");
  });
});

describe("setLanguage", () => {
  it("matches the base language and falls back to English", () => {
    registerCatalog("fr", {});
    expect(setLanguage("fr-CA")).toBe("fr");
    expect(setLanguage("ja-JP")).toBe("en");
    expect(availableLanguages()).toContain("en");
  });
});
