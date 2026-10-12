/**
 * Theme defaults (SPEC §10.2–10.5), shared with Rust through ts-rs. The bar resolves its own
 * theme in Rust (`bar/theme.rs`); Settings only needs the defaults to show them.
 */
import type { Theme } from "./bindings/Theme";
import type { ThemeDefaults } from "./bindings/ThemeDefaults";
import themeDefaultsJson from "./bindings/themeDefaults.json";

export const THEME_DEFAULTS = themeDefaultsJson as Record<Theme, ThemeDefaults>;
