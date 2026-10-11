import { describe, expect, it } from "vitest";
import type { Settings } from "./bindings/Settings";
import defaultSettings from "./bindings/defaultSettings.json";
import { applyTheme, fontFamily, fontStyleCss, fontWeight, themeTokens } from "./theme";

/** A fresh copy of `Settings::default()`, exported by the Rust tests. */
function defaults(): Settings {
  return structuredClone(defaultSettings as Settings);
}

const dark = { dark: true, accent: "#112233" };
const light = { dark: false, accent: "#112233" };

describe("themeTokens", () => {
  it("uses modern defaults", () => {
    const t = themeTokens(defaults(), dark);
    expect(t["--bar-height"]).toBe("28px");
    expect(t["--bar-bg"]).toBe("#1C1C1F");
    expect(t["--bar-bg-opacity"]).toBe("0.8");
    expect(t["--sensor-fg"]).toBe("#FFFFFF");
    expect(t["--sensor-weight"]).toBe("600");
    expect(t["--sensor-small"]).toBe("10px");
    expect(t["--popup-bg"]).toBe("#202024");
    expect(t["--popup-opacity"]).toBe("0.92");
    expect(t["--popup-radius"]).toBe("8px");
    expect(t["--gap"]).toBe("10px");
    expect(t["--accent"]).toBe("#112233");
    expect(t["--inactive-opacity"]).toBe("0.78");
  });

  it("uses classic defaults", () => {
    const s = defaults();
    s.theme = "classic";
    const t = themeTokens(s, dark);
    expect(t["--bar-height"]).toBe("25px");
    expect(t["--bar-bg"]).toBe("#2A2A2E");
    expect(t["--bar-bg-opacity"]).toBe("0.9");
    expect(t["--sensor-font"]).toMatch(/^"Arial"/);
    expect(t["--sensor-weight"]).toBe("700");
    expect(t["--sensor-small"]).toBe("9px");
    expect(t["--popup-bg"]).toBe("#000000");
    expect(t["--popup-radius"]).toBe("6px");
    expect(t["--bar-hairline"]).toBe("transparent");
  });

  it("applies user overrides", () => {
    const s = defaults();
    s.bar.height = 33;
    s.background = { colorMode: "custom", color: "#102030", opacity: 55, material: null };
    s.sensors.font = "Consolas";
    s.sensors.fontStyle = "boldItalic";
    s.popups.fontSize = 14;
    const t = themeTokens(s, dark);
    expect(t["--bar-height"]).toBe("33px");
    expect(t["--bar-bg"]).toBe("#102030");
    expect(t["--bar-bg-opacity"]).toBe("0.55");
    expect(t["--sensor-font"]).toMatch(/^"Consolas"/);
    expect(t["--sensor-weight"]).toBe("700");
    expect(t["--sensor-style"]).toBe("italic");
    expect(t["--popup-size"]).toBe("14px");
  });

  it("standard background colour mode ignores the custom colour", () => {
    const s = defaults();
    s.background.colorMode = "standard";
    s.background.color = "#FF0000";
    expect(themeTokens(s, dark)["--bar-bg"]).toBe("#1C1C1F");
  });

  it("follows OS light mode only when colours are not customised", () => {
    const t = themeTokens(defaults(), light);
    expect(t["--bar-bg"]).toBe("#F3F3F3");
    expect(t["--sensor-fg"]).toBe("#1A1A1A");

    const custom = defaults();
    custom.background.color = "#1C1C1F";
    expect(themeTokens(custom, light)["--bar-bg"]).toBe("#1C1C1F");
    expect(themeTokens(custom, light)["--sensor-fg"]).toBe("#FFFFFF");

    const text = defaults();
    text.sensors.textColor = "#FFCC00";
    expect(themeTokens(text, light)["--sensor-fg"]).toBe("#FFCC00");

    const classic = defaults();
    classic.theme = "classic";
    expect(themeTokens(classic, light)["--bar-bg"]).toBe("#2A2A2E");
  });
});

describe("fonts", () => {
  it("maps styles to weight and style", () => {
    expect([fontWeight("normal"), fontWeight("semibold"), fontWeight("bold"), fontWeight("italic")]).toEqual([400, 600, 700, 400]);
    expect([fontStyleCss("semiboldItalic"), fontStyleCss("bold")]).toEqual(["italic", "normal"]);
  });

  it("quotes family names safely", () => {
    expect(fontFamily('Bad"Font')).toBe('"BadFont", "Segoe UI", system-ui, sans-serif');
  });
});

describe("applyTheme", () => {
  it("writes tokens and the theme marker", () => {
    const el = document.createElement("div");
    applyTheme({ "--bar-height": "28px" }, "classic", el);
    expect(el.style.getPropertyValue("--bar-height")).toBe("28px");
    expect(el.dataset.theme).toBe("classic");
  });
});
