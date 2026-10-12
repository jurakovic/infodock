import { mount } from "svelte";
import SettingsApp from "./settings/SettingsApp.svelte";

// The Settings window is the only webview (SPEC §4.1); the bar is drawn natively.
mount(SettingsApp, { target: document.getElementById("app")! });
