import { mount, type Component } from "svelte";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { resolveApp, type AppKind } from "./lib/route";

function windowLabel(): string | null {
  try {
    return getCurrentWindow().label;
  } catch {
    return null; // plain browser: dev preview, screenshots
  }
}

const loaders: Record<AppKind, () => Promise<{ default: Component }>> = {
  bar: () => import("./bar/BarApp.svelte"),
  hint: () => import("./bar/BarApp.svelte"), // replaced in M1
  popup: () => import("./bar/BarApp.svelte"), // replaced in M3
  settings: () => import("./settings/SettingsApp.svelte"),
};

const kind = resolveApp(windowLabel(), location.pathname);
document.documentElement.dataset.app = kind;
const { default: App } = await loaders[kind]();
mount(App, { target: document.getElementById("app")! });
