/**
 * Theme tokens (SPEC §12.1): CSS custom properties on :root computed from the settings, the
 * theme defaults and the OS appearance. `themeTokens` is pure; `applyTheme` writes to the DOM.
 */
import type { FontStyle } from "./bindings/FontStyle";
import type { Settings } from "./bindings/Settings";
import type { Theme } from "./bindings/Theme";
import type { ThemeDefaults } from "./bindings/ThemeDefaults";
import themeDefaultsJson from "./bindings/themeDefaults.json";

export const THEME_DEFAULTS = themeDefaultsJson as Record<Theme, ThemeDefaults>;

/** Windows' default accent, used until the platform reports the real one. */
export const DEFAULT_ACCENT = "#0078D4";

export interface OsAppearance {
  dark: boolean;
  accent: string;
}

/** Modern theme in OS light mode, when the user has not customised colours (SPEC §12.2). */
const LIGHT = { barBg: "#F3F3F3", sensorFg: "#1A1A1A" } as const;
const DEFAULT_TEXT_COLOR = "#FFFFFF";

export type Tokens = Record<`--${string}`, string>;

export function fontWeight(style: FontStyle): number {
  switch (style) {
    case "semibold":
    case "semiboldItalic":
      return 600;
    case "bold":
    case "boldItalic":
      return 700;
    default:
      return 400;
  }
}

export function fontStyleCss(style: FontStyle): "normal" | "italic" {
  return style === "italic" || style === "semiboldItalic" || style === "boldItalic" ? "italic" : "normal";
}

/** A CSS font-family list: the chosen family, then safe fallbacks. */
export function fontFamily(name: string): string {
  return `"${name.replace(/["\\]/g, "")}", "Segoe UI", system-ui, sans-serif`;
}

export function themeTokens(s: Settings, os: OsAppearance): Tokens {
  const d = THEME_DEFAULTS[s.theme];
  const modern = s.theme === "modern";
  const customBackground = s.background.colorMode === "custom" && s.background.color !== null;
  const light = modern && !os.dark && !customBackground && s.sensors.textColor === DEFAULT_TEXT_COLOR;

  const themeBg = light ? LIGHT.barBg : d.backgroundColor;
  const barBg = s.background.colorMode === "custom" ? (s.background.color ?? themeBg) : themeBg;
  const sensorStyle = s.sensors.fontStyle ?? d.sensorFontStyle;
  const popupStyle = s.popups.fontStyle ?? d.popupFontStyle;

  return {
    "--bar-height": `${s.bar.height ?? d.barHeight}px`,
    "--bar-bg": barBg,
    "--bar-bg-opacity": String((s.background.opacity ?? d.backgroundOpacity) / 100),
    "--bar-hairline": modern ? (light ? "rgb(0 0 0 / 8%)" : "rgb(255 255 255 / 8%)") : "transparent",
    "--bar-radius": "0px",
    "--bar-pad": "7px",
    "--gap": `${s.sensorSpacing}px`,
    "--sensor-fg": light ? LIGHT.sensorFg : s.sensors.textColor,
    "--sensor-font": fontFamily(s.sensors.font ?? d.sensorFont),
    "--sensor-weight": String(fontWeight(sensorStyle)),
    "--sensor-style": fontStyleCss(sensorStyle),
    "--sensor-small": modern ? "10px" : "9px",
    "--sensor-medium": "11px",
    "--sensor-large": "15px",
    "--popup-bg": s.popups.background ?? d.popupBackground,
    "--popup-opacity": String((s.popups.opacity ?? d.popupOpacity) / 100),
    "--popup-fg": s.popups.textColor,
    "--popup-strong": s.popups.strongTextColor,
    "--popup-font": fontFamily(s.popups.font ?? d.popupFont),
    "--popup-weight": String(fontWeight(popupStyle)),
    "--popup-style": fontStyleCss(popupStyle),
    "--popup-size": `${s.popups.fontSize}px`,
    "--popup-radius": modern ? "8px" : "6px",
    "--popup-shadow": s.popups.shadow ? "0 4px 16px rgb(0 0 0 / 35%)" : "none",
    "--accent": os.accent,
    "--inactive-opacity": String(s.sensors.inactiveOpacity / 100),
    "--inactive-color": s.sensors.inactiveColor,
  };
}

/** Writes tokens to `el` (default :root) and marks the theme for theme-specific CSS. */
export function applyTheme(tokens: Tokens, theme: Theme, el: HTMLElement = document.documentElement): void {
  for (const [name, value] of Object.entries(tokens)) {
    el.style.setProperty(name, value);
  }
  el.dataset.theme = theme;
}
