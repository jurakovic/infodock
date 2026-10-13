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
- [ ] Settings ▸ General ▸ *Global hotkey*: select the button (it reads *Press keys…*), press Ctrl+Alt+K: the button shows `Ctrl+Alt+K` and the new hotkey works at once; the old one no longer does. Esc while recording keeps the old hotkey.
- [ ] While recording, press K alone or Shift+K: a message asks for Ctrl, Alt or Win and recording goes on. F9 alone is accepted.
- [ ] Record a combination another app already uses: the message names it, and the old hotkey keeps working. Quit the other app, then record the combination again: it is taken.
- [ ] Start Infodock a second time (Start menu): no second bar; the existing bar shows and Settings opens with keyboard focus.
- [ ] `taskkill /im infodock.exe` (without `/f`): Infodock quits and the reserved space is released.

**Appearance**

- [ ] Windows ▸ Personalisation ▸ Colours ▸ *Transparency effects* off: the bar turns plain (no blur) without a restart. On again: the blur returns.
- [ ] Windows ▸ Personalisation ▸ Colours ▸ *Choose your mode* → Light: the bar turns light grey; Dark: dark again. No restart needed.

## M1 — Framework

The bar is drawn natively since M1 S1, Settings uses Slint (plan D16) and the tray is native (D17); no browser engine runs at all (SPEC §4.1, §14.1). The M0 checks above still apply.

**Memory**

- [ ] Task Manager ▸ *Details*: `infodock.exe` is the only process (no `msedgewebview2.exe`), and its *Memory (active private working set)* is at most 30 MB.
- [ ] Open Settings (tray → *Settings…*), then close it: Infodock keeps running, and memory is back near the idle value.
- [ ] Hover and click the bar: the window you were working in keeps the focus.

**Sensors on the bar** (since M1 S4)

- [ ] With the first-run config, the bar shows the Infodock logo in the centre, and dimmed, outlined labels `cpu-graph`, `memory-meter`, `network-graph` on the left and `date`, `time` on the right: those sensor types arrive in M2.
- [ ] Settings ▸ General ▸ *Space between sensors*: the gaps on the left and right change at once.
- [ ] At 125 % and 150 % scaling, the logo and the labels are crisp, not blurred (an Image is drawn from a whole pixel since S12d).
- [ ] Add a Text sensor (*New sensor… ▸ Text*) with the text "Hello": it shows "Hello"; a Spacer with *Show divider* shows a faint vertical line; an Image with a picture file shows that picture one pixel less than the bar tall.
- [ ] Images (since S12d): a large photo (e.g. 24 megapixels) shows; an SVG and a compressed `.svgz` show. A picture over 64 megapixels is refused (the logo shows instead, and the log says why) without freezing or crashing Infodock.

**Menus and Lock** (since M1 S6)

- [ ] Right-click an empty part of the bar: *New sensor…*, *Tools ▸* (only *Layout manager…* enabled), *Settings…*, *Lock*, *Popups*, *Quit Infodock*. The check marks match the tray menu.
- [ ] Right-click the logo: *New sensor…*, *Properties…*, *Remove sensor*, *Tools ▸*, *Settings…*, *Lock*, *Popups*. On a placeholder (e.g. `cpu-graph`), *Properties…* is greyed out.
- [ ] *Remove sensor* on the logo asks "Remove sensor “Image”?"; *Cancel* keeps it, *Remove* removes it at once (and from `config.json`).
- [ ] Settings ▸ General ▸ *“New sensor” in the bar’s menu* → *Lists the sensor types*: *New sensor* becomes a submenu *General ▸ Image, Spacer, Text*; choosing *Text* adds a "Text" sensor after the one you clicked (or in the zone under the pointer) and opens its properties.
- [ ] Press and hold the left button on the logo: it dims; release: it is back.
- [ ] *Lock* on and Settings ▸ General ▸ *Clicks while locked* → *Only while Alt is held*: pressing the logo no longer dims it; with Alt held it does.

**Reordering** (since M1 S8)

- [ ] Drag the logo (press, move more than a few px): it stays dimmed and a thin accent-coloured line shows where it would go: between sensors, or after the left zone's last sensor when over empty space in the left third, before the right zone's first sensor in the right third. Release: the logo moves there, and the order is kept after a restart.
- [ ] Drag a placeholder onto the right half of its neighbour: they swap. Drop a sensor right next to itself: nothing changes.
- [ ] While dragging, press Esc: the drag ends and nothing moves. No hint appears during a drag.
- [ ] *Lock* on: dragging does nothing (and releasing elsewhere is no click).

**Hints** (since M1 S7)

- [ ] Rest the pointer on the logo: after about half a second a dark rounded hint "Infodock 0.1.0" appears above it, with a small triangle pointing at the logo and a soft shadow; it fades in quickly. Move away: it disappears after a moment.
- [ ] Rest on a placeholder (e.g. `cpu-graph`): its hint explains that the sensor is not available in this version. Near the screen's left edge the hint stays on screen and its triangle still points at the sensor.
- [ ] With a hint shown, move straight to another sensor: the hint switches at once, without a new delay.
- [ ] Click a sensor while its hint shows: the hint closes and does not come back until the pointer moves to another sensor. Right-click: the hint closes before the menu opens.
- [ ] Tray → *Popups* off: no hints. On again: hints are back.
- [ ] Bar at the top (Settings ▸ Bar ▸ Screen edge): hints appear below the bar, their triangle pointing up.
- [ ] With the logo's hint shown, add a sensor in front of it from Settings (picker, *Add*): the hint moves with the logo (since S12c).
- [ ] At 150 % scaling the hint text is crisp.
- [ ] Settings ▸ Popups ▸ *Delay before a popup shows* at 0: hints appear at once.

**Floating mode** (since M1 S11)

- [ ] Settings ▸ Bar ▸ *Position* → *Floating*: the bar becomes an 800 px wide strip, centred a little above the bottom of the screen; maximised windows no longer stop at it, and *Reserve screen space* is hidden. *Width* changes its width at once.
- [ ] Drag the bar by an empty part of its background: it follows the pointer smoothly; release it: after a restart it is in the same place. Dragging a sensor still reorders sensors.
- [ ] Drag it partly off the screen and release: it slides back inside the screen. With two monitors, drag it onto the other one (also one with another scale): it stays there with the right height. Dropped across the boundary, it goes to the monitor holding most of it (since S12c).
- [ ] *Lock* on: dragging the background does nothing.
- [ ] Hints: with the bar near the top of the screen they open below it, near the bottom above it. The bar's thin edge line is on the side facing the screen centre (below the bar near the top).
- [ ] *Position* → *Docked*: the bar is back at its edge, reserving space again.

**Highlight and fades** (since M1 S11)

- [ ] Start Infodock: the bar slides up from below the screen edge while its content fades in (about a third of a second). With the bar at the top, it slides down from above.
- [ ] Settings ▸ Bar ▸ *Screen edge* → *Top*: the bar slides down out of the screen, then slides in at the top. Back to *Bottom*: the same the other way. Maximised windows follow the reserved space.
- [ ] Change the edge to *Top* and at once back to *Bottom* (within a third of a second): the bar slides out and comes back; it never stays invisible (since S12c).
- [ ] While the bar rises, another app bar or a display change moves things: the bar does not jump (since S12c).
- [ ] *Animations* → *Off*, then change the edge or restart: the bar is simply there, without sliding or fading.

- [ ] Settings ▸ Bar ▸ *Highlight on hover* on: moving the pointer onto the bar makes its background noticeably more opaque over about a third of a second; leaving fades it back.
- [ ] Add a Spacer without divider (*New sensor… ▸ Spacer*): hovering it fades in a faint vertical line; leaving fades it out. With *Show divider* on, the line is always there.
- [ ] *Animations* → *Off*: the highlight and the Spacer's line appear and disappear at once. *Like Windows* follows Settings ▸ Accessibility ▸ Visual effects ▸ *Animation effects* at once (since S12c).

**Sensor properties** (since M1 S9)

- [ ] Right-click the logo → *Properties…*: Settings opens "Image properties" with *Image file*, *Colour*, *Opacity* and *Popup text*.
- [ ] *Colour* → *Custom* and a colour such as `#20B7FF` (Enter): within a moment the logo on the bar turns that colour; nothing is saved yet. *Opacity* 50 %: the logo fades.
- [ ] *Browse…* opens the Windows file picker filtered to images; pick a PNG: the bar shows it. Type into the path first, then *Browse…*: the box shows the picked path (since S12a2).
- [ ] Type `#12345` as the custom colour: the row says how to write a colour and *Save* is greyed out; complete it to `#123456`: the message goes and *Save* works (since S12b).
- [ ] *Cancel*: the logo is back as it was. Again with *Save*: the change stays, also after a restart.
- [ ] Change something, then click *General* in the navigation, or close Settings: the change is dropped.

**New sensor** (since M1 S9)

- [ ] Right-click the logo → *New sensor…*: Settings opens "New sensor" with the search box focused, *Insert After the “Image” sensor*, and *General ▸ Image, Spacer, Text*, each with its description. *Add* is greyed out until a type is selected.
- [ ] Type `line`: only *Text* (and *Spacer*, whose description mentions a line) remain; type `xyz`: "No sensor matches the search."
- [ ] Choose *Before*, double-click *Text*: "Text properties" opens and a "Text" sensor appears left of the logo. *Cancel*: it is gone again. Again with *Save*: it stays.
- [ ] Right-click empty space in the right third → *New sensor…*: there is no Insert choice; *Add* puts the sensor at the start of the right zone (next to the centre). In the left third, at the end of the left zone.
- [ ] Open the picker, then click *Sensors* in the navigation: nothing is added.
- [ ] Keyboard only (since S12b): open the picker, type `e`: *Image* is selected; Down and Up in the search box move the selection; Enter adds the selected type.

**Layout manager** (since M1 S9)

- [ ] Right-click the bar → *Tools ▸ Layout manager…*: Settings opens on *Sensors* with *Layout* at the top; *Left*, *Centre* and *Right* list the bar's sensors in order. The logo's row reads *Image* with *Infodock logo* below it; placeholders read "Not available in this version" and their *Properties* button is greyed out. The sensor defaults follow below the list.
- [ ] *Move down* on the logo: it moves to the start of the right zone, on the bar too. *Move up* brings it back. The first sensor's *Move up* and the last one's *Move down* are greyed out.
- [ ] Drag a row: it dims and an accent line shows where it will go; release: the bar follows. Drag sideways out of the list and release: nothing moves.
- [ ] Drag a sensor on the bar: the list in Settings follows at once.
- [ ] *Remove* on a sensor: the row asks "Remove sensor “…”?"; *Cancel* keeps it, *Remove* removes it from the list and the bar.
- [ ] *Add…* next to *Centre*: the picker opens without an Insert choice; *Add* puts the new sensor at the end of the centre zone and opens its properties.
- [ ] Keyboard: Tab reaches every button of every row, Space activates it; Narrator reads each button's name (*Move up*, *Move down*, *Properties*, *Remove*). After *Move down* with Space, the focus stays on that sensor's *Move down*, so Space again moves it on (since S12b).

**Settings window (Slint)**

- [ ] It looks like Windows 11 Settings: navigation on the left; each setting with its label and help on the left and its control on the right. Light and dark mode follow Windows.
- [ ] Every control from the M0 checks works the same (hotkey recording, choices, switches, sliders save when released, *Use theme default*). A tray toggle of *Lock* or *Popups* shows up in Settings at once.
- [ ] Since S12a: a slider shows the value it will save (e.g. *Delay before a popup shows* in steps of 50 ms) and snaps to it when released. Tabbing through *Popups ▸ Background* or clicking its opacity slider without moving it keeps *Use theme default* greyed out. Typing a colour and clicking *General* saves it. Recording the hotkey, then clicking another page ends recording.
- [ ] Record a hotkey that another app uses: the error shows; close and reopen Settings: no error (the old hotkey still works).
- [ ] Ctrl+Space in the navigation does not blank the page.
- [ ] Keyboard: Tab moves through the controls, Space toggles switches, arrow keys change choices and sliders. Narrator reads each control with its label.
- [ ] Resize the window down to its minimum: labels and help texts wrap, nothing overlaps or is cut off.
- [ ] With Settings open behind other windows, start Infodock again: Settings comes to the front.

**Tray (native)**

- [ ] Right-click the tray icon: the menu opens above the pointer; *Lock* and *Popups* show check marks matching Settings ▸ General. Toggle each: the mark flips the next time the menu opens, and Settings follows.
- [ ] The menu closes when clicking elsewhere or pressing Esc. Keyboard: Win+B, arrow to the icon; Enter or Space toggles the bar exactly once (Explorer reports Enter twice; the duplicate is dropped), Shift+F10 or the menu key opens the menu.
- [ ] Hover the icon: the tooltip reads *Infodock*.
- [ ] Restart Explorer (Task Manager ▸ *Windows Explorer* ▸ Restart): the tray icon comes back.
- [ ] Quit from the tray: the icon disappears at once (no ghost icon until hovered).
