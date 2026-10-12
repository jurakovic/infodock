# Manual test checklist

Run on real Windows 10 and Windows 11 machines (SPEC §16). Each milestone adds a section. When something fails, note the step and attach the log from `%LOCALAPPDATA%\app.infodock.desktop\logs\` (newest `infodock.*.log`).

## Getting a build

1. Open the latest green run of [CI](https://github.com/jurakovic/infodock/actions/workflows/ci.yml) for `master`.
2. Download the `infodock-windows-installer` artifact, unzip it and run the `infodock_*_x64-setup.exe` inside. It installs per user, without admin rights. It is not signed yet, so SmartScreen may ask: *More info → Run anyway*.
3. Files: config `%APPDATA%\app.infodock.desktop\config.json`, logs `%LOCALAPPDATA%\app.infodock.desktop\logs\`.

To start over from first run, quit Infodock and delete `%APPDATA%\app.infodock.desktop`.

## M0 — Skeleton

Exit criteria: the bar reserves space correctly on 1–2 monitors at 100/150 % DPI, survives an Explorer restart, and a clean exit restores the work area. The bar is empty in M0 (sensors arrive in M1–M2).

**Start-up and placement**

- [ ] First start: a dark, slightly translucent (Acrylic) bar appears along the bottom of the primary monitor, full width, above the taskbar. At 100 % scaling it is 28 px tall.
- [ ] Maximise any window: it stops at the bar instead of covering it.
- [ ] Alt+Tab does not list Infodock. The taskbar shows no button for it.
- [ ] Win+Left/Right, or dragging the bar: it does not move.
- [ ] Click the bar, then press Alt+F4: nothing happens (the bar stays; quitting is in the tray menu).
- [ ] With the bar unfocused (click another window), it still looks translucent rather than solid.

**Settings ▸ Bar** (tray icon → *Settings…* → *Bar*)

- [ ] *Screen edge* → Top: the bar moves to the top; maximised windows now start below it. Back to Bottom.
- [ ] *Height* slider: the bar's height changes and maximised windows follow. *Use theme default* returns to 28.
- [ ] *Reserve screen space* off: maximised windows extend under the bar (the bar stays on top). On again: they stop at the bar.
- [ ] *Stacking* → *Like other windows*: clicking another window brings it in front of the bar where they overlap. → *Always behind other windows*: windows always cover the bar, but on an empty desktop the bar is still visible above the wallpaper; the hotkey (Ctrl+Alt+Shift+I) brings it forward until you click elsewhere. Back to *Always on top*.
- [ ] Full-screen: play a video full-screen (or press F11 in a browser) on the bar's monitor: the bar steps behind it. Leave full-screen: the bar is back on top. With *Step back for full-screen apps* off, the bar stays on top.

**Monitors and DPI** (two monitors, one at 100 % and one at 150 % if possible)

- [ ] *Monitor* lists both monitors with their names and resolutions. Pick the second: the bar moves there and reserves space there; the first monitor's work area is back to normal.
- [ ] On a 150 % monitor the bar is 42 physical px tall (28 × 1.5) and crisp.
- [ ] Change that monitor's scaling (Settings ▸ Display ▸ Scale) while Infodock runs: the bar resizes and still reserves the right amount of space.
- [ ] Disconnect the monitor the bar is on: the bar moves to the primary monitor. Reconnect it: the bar returns.
- [ ] Change the resolution or rearrange the monitors: the bar follows.

**Explorer restart, exit and crash**

- [ ] In a terminal: `taskkill /f /im explorer.exe`, then `start explorer.exe`. When the taskbar is back, maximise a window: it still stops at the bar.
- [ ] Tray → *Quit Infodock*: the bar disappears and a maximised window now fills the space the bar used.
- [ ] Start Infodock, then end it from Task Manager (*End task*): the reserved space is released as well.

**Tray, hotkey, single instance**

- [ ] Left-click the tray icon: the bar hides and its space is released. Click again: it is back. The menu's first item reads *Hide bar* / *Show bar* accordingly.
- [ ] Tray → *Lock* and *Popups*: the check marks toggle and match Settings ▸ General (and the other way round).
- [ ] Settings ▸ General ▸ *Show the tray icon* off: the icon disappears. Ctrl+Alt+Shift+I brings the bar back if it was hidden.
- [ ] Hide the bar from the tray, then press Ctrl+Alt+Shift+I: the bar reappears.
- [ ] *Hotkey action* → *Show or hide the bar*: the hotkey now toggles the bar.
- [ ] Set the hotkey to something invalid (e.g. `Ctrl+`) and press Enter: an error appears under the field and the old hotkey keeps working. Set a combination another app already uses: an error appears too.
- [ ] Start Infodock a second time (Start menu): no second bar; the existing bar shows and Settings opens with keyboard focus.
- [ ] `taskkill /im infodock.exe` (without `/f`): Infodock quits and the reserved space is released.

**Appearance**

- [ ] Windows ▸ Personalisation ▸ Colours ▸ *Transparency effects* off: the bar turns plain (no blur) without a restart. On again: the blur returns.
- [ ] Windows ▸ Personalisation ▸ Colours ▸ *Choose your mode* → Light: the bar turns light grey; Dark: dark again. No restart needed.

## M1 — Framework (in progress)

The bar is drawn natively since M1 S1, Settings uses Slint (plan D16) and the tray is native (D17); no browser engine runs at all (SPEC §4.1, §14.1). The M0 checks above still apply.

**Memory**

- [ ] Task Manager ▸ *Details*: `infodock.exe` is the only process (no `msedgewebview2.exe`), and its *Memory (active private working set)* is at most 30 MB.
- [ ] Open Settings (tray → *Settings…*), then close it: Infodock keeps running, and memory is back near the idle value.
- [ ] Hover and click the bar: the window you were working in keeps the focus.

**Settings window (Slint)**

- [ ] It looks like Windows 11 Settings: navigation on the left; each setting with its label and help on the left and its control on the right. Light and dark mode follow Windows.
- [ ] Every control from the M0 checks works the same (hotkey with Enter, choices, switches, sliders save when released, *Use theme default*). A tray toggle of *Lock* or *Popups* shows up in Settings at once.
- [ ] Keyboard: Tab moves through the controls, Space toggles switches, arrow keys change choices and sliders. Narrator reads each control with its label.
- [ ] Resize the window down to its minimum: labels and help texts wrap, nothing overlaps or is cut off.
- [ ] With Settings open behind other windows, start Infodock again: Settings comes to the front.

**Tray (native)**

- [ ] Right-click the tray icon: the menu opens above the pointer; *Lock* and *Popups* show check marks matching Settings ▸ General. Toggle each: the mark flips the next time the menu opens, and Settings follows.
- [ ] The menu closes when clicking elsewhere or pressing Esc. Keyboard: Win+B, arrow to the icon; Enter or Space toggles the bar exactly once (Explorer reports Enter twice; the duplicate is dropped), Shift+F10 or the menu key opens the menu.
- [ ] Hover the icon: the tooltip reads *Infodock*.
- [ ] Restart Explorer (Task Manager ▸ *Windows Explorer* ▸ Restart): the tray icon comes back.
- [ ] Quit from the tray: the icon disappears at once (no ghost icon until hovered).
