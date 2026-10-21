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

## M2 — System sensors

The first-run placeholders are real sensors now. The M0 and M1 checks above still apply. CI renders every system sensor with made-up data (`bar-system*.png` in `infodock-screenshots`) and measures the idle CPU; the checks here cover what only real hardware shows.

**Time** (since M2 S6)

- [ ] The first-run *time* placeholder is now a clock: the hour large, the minutes small and raised to its right; on an en-US system with *am*/*pm* under the minutes (a 12-hour clock), on a 24-hour regional format without. It turns to the next minute within a moment of the system clock.
- [ ] Properties: *24-hour clock* switches the format; *Show seconds* shows `H:MM` large with the seconds small, updating every second; the bar does not shift as the digits change (it may when the hour gains a digit).
- [ ] Hover: the hint shows today's long date in the regional format. Click: Settings ▸ Time & language ▸ Date & time opens.
- [ ] *Chime* → *Every 15 minutes*: a soft two-tone chime plays at :00, :15, :30 and :45, not at other minutes, and not right after starting Infodock or waking the computer.
- [ ] Change the time zone or the system time: the clock follows at once. Sleep and wake the computer: the clock is right at once.

**Date** (since M2 S7)

- [ ] The first-run *date* placeholder is now the date: the day large (`Tuesday, 13`) with the year above the month (`2026` / `october`) to its right.
- [ ] Properties: each weekday, month and year choice changes the view as described (e.g. *Year* → *None* gives one line in the regional order, such as `Tuesday, October 13`); with a German regional format the names are German.
- [ ] Hover: a month calendar from Monday (or Sunday, per *Calendar*), weekends in the strong popup colour, today highlighted, and `Week …, Day …` below; with *Calendar* → *None*, only that line.
- [ ] Click: the date and time settings open; with *Settings ▸ Applications ▸ Calendar* set (when that page exists), that app.
- [ ] Just after midnight (or after changing the date), the date shows the new day.

**Uptime** (since M2 S8)

- [ ] *New sensor… ▸ Info ▸ Uptime*: it shows `Uptime: 3d 04h 12m` (days only when there are any), matching Task Manager ▸ Performance ▸ *Up time*, and turns to the next minute on time.
- [ ] Hover: the uptime in words. After a restart of the computer, the hint also shows *Longest uptime* from before (it is stored at most 10 minutes behind).
- [ ] Properties: *Text before* empty shows only the uptime; font, style, size and shadow work as for Text.

**CPU** (since M2 S9)

- [ ] The first-run *cpu-graph* placeholder is now a CPU graph: a 50 px plot scrolling left once a second with green columns, and `CPU` / `37%` beside it. Its value stays within 3 % of Task Manager ▸ Performance ▸ CPU *Utilization* (open both side by side, load the CPU with a build or a video).
- [ ] Hover: `8 CPUs use` (your count of logical processors), activity, user, system and idle with one decimal, and *Click to open Task Manager*; click: Task Manager opens.
- [ ] *New sensor… ▸ System ▸ CPU Meter*: a thin bar filling with the load and `CPU` / `37%`; *Colour* → *Dynamic* turns it green, yellow and red with the load.
- [ ] Graph properties: *Width* 50–150 in steps of 5; background and grid colours and opacities (both off by default); *Graph colour* → *Dynamic* colours each column by its value; *Update every* 1–10 s slows the scrolling.
- [ ] After sleep and wake, the first values are sensible (no spike or drop to 0).
- [ ] *New sensor… ▸ System ▸ CPU Graph (dual)*: processor 1 in blue growing up from the middle, processor 2 in orange growing down; `CPUs` / `37%` beside them. Properties list *Total*, *User tasks*, *System* and each of your processors for *Top shows* and *Bottom shows*; *Bottom grows down* off makes the orange graph grow up from the bottom edge.

**Memory** (since M2 S11)

- [ ] The first-run *memory-meter* placeholder is now a meter: `RAM` / `43%`, its bar filled to the same share. The value stays within 3 % of Task Manager ▸ Performance ▸ Memory (*In use* of the total); load memory (open many browser tabs) and watch both.
- [ ] Hover: *Total Physical*, *Available Physical* and *Memory Load* (Total a little below the installed size, as Windows reserves some), and *Click to open Task Manager*; click: Task Manager opens.
- [ ] *New sensor… ▸ System ▸ Memory Graph*: a graph of the load with `RAM` / `5.21 GB` (the used size); *Show* → *Percentage in use* shows `43%` instead; the plot is the same in both. Graph properties behave as for the CPU graph.
- [ ] Meter properties: *Update every* 1–60 s (2 s by default); *Colour* → *Dynamic* goes from green to red with the load.

**Swap** (since M2 S12)

- [ ] *New sensor… ▸ System ▸ Swap Meter* and *Swap Graph*: `Swap` / the share of the page file in use. Hover: *Total Virtual* is the page file's current size (System Properties ▸ Advanced ▸ Performance ▸ Virtual memory, *Currently allocated*), *Available Virtual* and *Swap Load*; click: Task Manager.
- [ ] With the page file turned off (*No paging file*, restart): both show `n/a`, their bar or graph dimmed (the label stays), and the hint says *No page file configured*.

**Disk Meter** (since M2 S13)

- [ ] *New sensor… ▸ System ▸ Disk Meter*: `HD: C:\` with the used share, within 1 % of Explorer ▸ This PC (the bar shows the used fraction). Hover: *Disk Meter for C:\*, *Total Space*, *Free Space* and *Disk Space Used*; click: the drive opens in Explorer.
- [ ] Properties: *Drive* lists your drives by label or kind (`Local Disk (C:)`), without optical drives; *Show* → free percent, used size and free size (the sizes under the label); a *Title* replaces `HD: C:\`; *Update every* 10–300 s.
- [ ] A Disk Meter on a mapped network drive whose server is off (or disconnect from the network): the drive shows `n/a` after a while, and meanwhile the CPU and network graphs keep moving (the sampler is not held up). Opening its properties does not freeze Settings.
- [ ] Pick a USB stick, then remove it: within the interval the meter shows `n/a` with its bar dimmed, the hint says the drive is not available, a click does nothing, and no "insert a disk" dialog appears. The properties keep it as `E:\ (not connected)`. Plug it back in: values return.

**Network** (since M2 S14)

- [ ] The first-run *network-graph* placeholder is now a network graph: receive in blue growing up from the middle, send in orange growing down, and the two rates beside it (`1.24 MB/s` over `86.0 KB/s`). During a large download the receive rate matches Task Manager ▸ Performance ▸ Wi-Fi/Ethernet *Receive* (and Resource Monitor ▸ Network ▸ *Network Activity* in total) within a few percent.
- [ ] The graphs scale to the largest rate in view (at least 1 KB/s); hover: *Adapter: Wi-Fi (Intel(R) …)*, *Status: active*, receive and send with *max.* (the scale in use), and *Click to open Network Connections*; click: Network Connections opens.
- [ ] Properties: *Connection* lists *Automatic* and your connections (no loopback); pick one, save, and the graph shows only it. *Maximum* `12500000` fixes both scales at 100 Mbit/s (`max. 11.9 MB/s`), `12500000,2500000` each; the colours, opacities and *Send grows down* work as for the CPU dual graph.
- [ ] Automatic follows the connection to the internet: switch from Wi-Fi to Ethernet (or connect a VPN) and the hint's adapter changes within a second, without a spike. Unplug the chosen adapter: *Status: inactive* and 0 rates; the properties keep it as `… (not connected)`.
- [ ] After sleep and wake, no spike in the first values.

**Budgets** (M2 S15)

- [ ] Task Manager ▸ Details, *CPU* column, the first-run layout: Infodock stays at 0–1 % on a 4-core machine (§14.1: ≤ 0.5 % on average over a minute; CI measures it too).
- [ ] *Memory* column: within the budget at idle, and still after the first chime, a click on the CPU graph (Task Manager opens) and on the Date (date settings open): the audio and shell libraries those load must not push it over 30 MB.

**Close-out fixes** (M2 S15)

- [ ] Uptime turns to the next minute on time, also after hovering it for a while (its hint updates too).
- [ ] A CPU Graph with *Update every* 5 s next to a 1 s CPU Meter: the meter changes every second, the graph moves one column every 5 s.
- [ ] Settings ▸ Time & language ▸ Language & region ▸ *Regional format* → German (Germany), without restarting Infodock: the bar's sizes and rates use a comma (`5,21 GB`, `1,24 MB/s`), and Date shows German names at once. Back to your format: back at once.
- [ ] Lock on with *Clicks when locked* → *Alt only*: the CPU, memory and network hints have no *Click to …* line; with Lock off, they have it again.
- [ ] *New sensor…* (picker): with every type listed, the search box and the Add and Cancel buttons stay in view; Up and Down move the selection and the list scrolls with it.
- [ ] Edit a Time sensor's properties: the clock does not blink empty while previewing.
- [ ] A Network Graph set to a connection that is then renamed (Network Connections ▸ Rename): its properties still show it (by the new name), not *Automatic*.

## M3 — Launchers & hardware

Exit criteria (SPEC §17): every §9.14–9.22 behaviour below passes; dragging a link from Edge or Chrome creates a web Shortcut, or a `.url` file in a Folder Menu.

The M0–M2 checks above still apply. CI renders the launchers (`bar-launchers*.png` in `infodock-screenshots`) with this machine's shell icons and the popups (`popup-folder*`, `popup-start-menu`, `popup-eject`); the checks here cover clicking, the shell, drag & drop and real devices.

**Shortcut** (since M3 S1)

- [ ] *New sensor… ▸ Launchers ▸ Shortcut*, *Target* → *Browse…* → `C:\Windows\System32\notepad.exe`, Save: Notepad's icon, 24 px on the 28 px bar, sharp at 100 % and 150 %. Hover: *Open 'notepad'*; click: Notepad opens.
- [ ] A `.lnk` from the Start Menu folder as the target shows its program's icon (no shortcut arrow); a folder shows the folder icon and opens in Explorer; a document opens in its app.
- [ ] *Parameters* `/A C:\Windows\win.ini` on Notepad opens that file; parameters with spaces in quotes arrive as one. *Start in* empty: a `.cmd` target that runs `cd` shows its own folder; set to another folder, that one.
- [ ] *Run as* → *Administrator*: the elevation prompt appears; *No* leaves nothing behind (no error dialog).
- [ ] *Web address* `https://example.com/` (with a target set too): the globe icon, hint *Open 'https://example.com/'*, the default browser opens the page. `ms-settings:display` opens the display settings.
- [ ] *Title* → `Editor`: the hint is just *Editor*. *Colour* → *Custom* tints the icon; *Opacity* 40 % fades it.
- [ ] *Image file*: an `.ico`, `.png` or `.svg` replaces the icon, keeping its aspect ratio.
- [ ] A target on a USB stick: remove the stick; within a minute the icon is the generic app icon, dimmed (inactive). Plug it back in: within a minute its own icon returns, not dimmed. No "insert a disk" dialog appears.
- [ ] A target on a network share whose server is off: the bar keeps responding (the CPU graph keeps moving) while the icon and the check wait; the icon space stays free, then shows the generic icon dimmed.
- [ ] Lock on with *Clicks when locked* → *Alt only*: the hint is only the title.

**Sensor menus and notifications** (since M3 S2)

- [ ] Right-click a sensor without its own items (e.g. the CPU graph): *New sensor…*, *Properties…*, *Remove sensor*, one separator, *Tools*, *Settings…*, *Lock*, *Popups*, as before.
- [ ] Quit Infodock, put a few random characters at the start of `%APPDATA%\app.infodock.desktop\config.json`, start it: a toast from *Infodock* says the settings could not be read and the last good copy was restored, naming the `config.broken-….json` file. Delete `config.json.bak` too and repeat: the toast says the defaults are used.
- [ ] The same with the portable `infodock.exe` on a machine where Infodock was never installed: the toast shows too (named *Infodock*).
- [ ] The Settings window's taskbar button groups with a pinned Infodock shortcut (installed build).
- [ ] `%LOCALAPPDATA%\app.infodock.desktop\logs`: the start-up log lists the capabilities. A laptop: `battery: true` and `wifi: true` (with Wi-Fi hardware); a desktop without Wi-Fi: `wifi: false`.

**Power** (since M3 S3)

- [ ] *New sensor… ▸ Launchers ▸ Power*: the sleep icon (21 px); hover: *Sleep*. Properties: *Action* lists Lock, Sign out, Switch user, Sleep, Hibernate (only with hibernation turned on: `powercfg /hibernate on`), Shut down, Restart; each changes the icon. *Close apps without asking* shows only for Sign out, Shut down and Restart.
- [ ] Click with *Ask first* on: a dialog asks "Do you really want to … this computer?" with the action on its button; Cancel and Esc do nothing. With *Ask first* off, the click acts at once.
- [ ] Each mode does what it says: Lock shows the lock screen; Sign out signs out (open Notepad with unsaved text: Windows asks, unless *Close apps without asking*); Switch user shows the sign-in screen and the session is still there after signing in again; Sleep sleeps (on a Modern Standby laptop: the screen turns off and it goes to standby), and the bar is fine after waking; Hibernate hibernates; Shut down and Restart do so (Notepad with unsaved text: Windows waits for it, unless forced).

**Volumes and Disk** (since M3 S4)

- [ ] *New sensor… ▸ Launchers ▸ Disk*: the first drive's icon as Explorer shows it; hover: *Open 'Local Disk (C:)'* (or its label); click: the drive opens in Explorer. *Drive* lists every drive, optical ones too; a *Title* replaces the name in the hint.
- [ ] A Disk on a USB stick: remove the stick, and within a second or two the icon is the generic drive icon, dimmed, the hint says *E:\ is not connected*, and a click does nothing. Plug it back in: its icon and name return within a second or two.
- [ ] A Disk on an empty DVD drive: not dimmed, hint *CD Drive (D:) — no media*; insert a disc: the hint shows the disc's label and the icon may change.
- [ ] A Disk Meter on the USB stick: removing and re-inserting the stick shows `n/a` and the values again within a second or two (not only at its interval).
- [ ] A Shortcut whose target is on the USB stick: it dims and comes back with the stick within a second or two.
- [ ] Open a Disk Meter's properties, then plug a USB stick in: the *Drive* list gains it without reopening the form, and the chosen drive stays chosen.
- [ ] Map a network drive (`net use Z: \\server\share`) and disconnect it: a Disk on Z: follows.

**Eject** (since M3 S5)

- [ ] Right-click a Disk on a USB stick: *Eject* sits between *Remove sensor* and *Tools*. Choose it: within a few seconds a toast *STICK (E:) can be safely removed.*, the Disk dims, and Explorer no longer lists the drive. The stick's light goes off (or Windows' *Safely Remove Hardware* no longer lists it).
- [ ] Open a file on the stick in Notepad (or `cd` into it in a Command Prompt), then *Eject*: a toast says it is in use (naming the program when Windows does), and the stick stays.
- [ ] A USB hard disk (Windows shows it as a local disk): *Eject* is offered and works; an internal disk's Disk has no *Eject*.
- [ ] A DVD drive with a disc: *Eject* opens the tray and the toast says it can be removed; with a file from the disc open, it is in use.
- [ ] An SD card in a laptop's built-in reader: *Eject* removes the card (the reader stays), with the toast.
- [ ] No Windows dialog of its own ("Problem Ejecting USB Mass Storage Device") appears in any of these.

**Recycle Bin** (since M3 S6)

- [ ] *New sensor… ▸ System ▸ Recycle Bin*: the bin icon with *Recycle Bin* above *empty*, *1 item* or *12 items*; the icon shows papers while it holds something. CI renders it (12 items) in `bar-system*.png`.
- [ ] Delete a file in Explorer: within the interval (10 s by default) the count goes up; restore it from the Recycle Bin and the count goes down.
- [ ] Hover: *Recycle Bin*, *Items: 12*, *Space used: 431 MB* (as Explorer's Recycle Bin properties count them), *Click to open the Recycle Bin*. Click: the Recycle Bin opens in Explorer.
- [ ] Right-click: *Open Recycle Bin* and *Empty Recycle Bin* between *Remove sensor* and *Tools*. *Empty Recycle Bin* asks as Windows does (unless confirmation is off in the Recycle Bin's properties), plays the sound, and the sensor shows *empty* within a second, not at the next interval. Cancelling leaves it as it was. With the bin empty, the item is greyed out.
- [ ] *Image when empty* / *Image when full* replace the icons; *Colour* → *Custom* tints the built-in ones; *Opacity* fades the icon only. A light bar (Windows light mode): the built-in icon is dark.
- [ ] A computer with a large Recycle Bin (thousands of items) or a sleeping second disk: the bar keeps responding while the count is read.

**Battery** (since M3 S7; a laptop)

- [ ] *New sensor… ▸ System ▸ Battery*: the battery icon (19×23) with its level coloured red → yellow → green by charge, the state above the percentage: *In Use 73%* on battery, *Low Battery* below 15 %, *Charging*, *Full*. The charging bolt and the charged plug show over the level; the level's top edge is lighter except when nearly full. CI renders it (73 %, and an absent second battery) in `bar-system*.png`.
- [ ] Unplug and plug the charger: the state changes within a second or two, not only every 10 s. The hint follows: name, *Capacity is 73%*, *2:41 Remaining* (or *Calculating Time Remaining*), and after unplugging while Infodock runs, *0:05 Already In Use*, counting up.
- [ ] Click on battery: the value switches to the time left (`~ 2:41`, `*` while unknown) and stays so after restarting Infodock; Settings shows *On battery, show* → *Time left*. Click again: back to the percentage. While charging the click switches *While charging, show* between the percentage and the time until full.
- [ ] *Smooth the time left* on: the time left moves smoothly instead of jumping when the load changes (start and stop a video).
- [ ] *Time until full* → *Calculated*: while charging, `*` until the charge has grown a percent, then a time that settles; it survives a click on the sensor (the estimate is not reset).
- [ ] *Charged at* 80 % with Windows' battery care (or a vendor charge limit) holding at 80 %: *Full*, no percentage. With *As Windows reports it*: *Full 80%*.
- [ ] *Tell me to unplug above* 90 %: charging past 90 % shows one toast *Battery at 91% — you can unplug the charger*; unplug and plug in again above 90 %: one more.
- [ ] Properties ▸ *Battery* lists `1. {vendor model}` for each battery; a laptop with two batteries shows both, and the second one's sensor shows its own charge. A desktop: the sensor is not offered (no battery capability).

**Wi-Fi** (since M3 S8; a computer with Wi-Fi)

- [ ] *New sensor… ▸ Network ▸ Wi-Fi Signal*: the signal icon with the network name above `78%`; the icon's arcs follow the signal (walk away from the router). *Show* → *Signal in dBm*: `-58 dBm`; → *Network name*: the percentage (the name is above already), unless a *Title* is set. CI renders it in `bar-system*.png`.
- [ ] Hover: *Wi-Fi Signal*, *Network: HomeNet*, *Info: 802.11ax, 5 GHz, channel 36, 1201 Mbps* (compare `netsh wlan show interfaces`), *Signal level*, *Signal percentage*, *Click to open the Wi-Fi settings*; the click opens them.
- [ ] Disconnect from the network: within a second the value is *offline* and the icon faint and crossed out (inactive); the hint says *Offline*. Reconnect: back within a second or two. Airplane mode: *Wi-Fi is turned off*.
- [ ] Windows 11 24H2 or later, Settings ▸ Privacy & security ▸ Location off (or *Let desktop apps access your location* off): the label is *Wi-Fi*, the signal still shows, the hint adds *Allow location access to show the network name*, and the click opens the location settings. Turning location access on brings the name back.
- [ ] A computer without Wi-Fi: *Wi-Fi Signal* is not offered in the picker.

**Popup window and Folder Menu** (since M3 S9 and S10a)

- [ ] *New sensor… ▸ Launchers ▸ Folder Menu*, *Folder* → your Downloads folder: the folder icon (21 px); hover: *Downloads*. Click: a list opens above the bar (below a top bar), its left edge at the sensor's (its right edge when there is no room), fading in; the sensor shows a faint highlight behind it while the list is open. CI renders it (`popup-folder*.png`, `C:\Windows`).
- [ ] The list: the header *Downloads* with a 2 px light-blue line under it, folders first, then files, sorted as Explorer sorts them (`File 2` before `File 10`), each with its shell icon; shortcuts without `.lnk`/`.url`; no `desktop.ini`, no hidden files (with *Show hidden files* on, they show). An empty folder says *no items*; a folder on a server that is away says *Loading…*, then that it cannot be opened, and the bar keeps responding.
- [ ] Hover highlights rows (light blue on the dark popup, dark blue on a light one); a click opens the file and closes the list; Alt+click shows it selected in Explorer; a click on the header opens the folder shown (Alt: reveals it).
- [ ] Keys while the list is open (no click needed first): ↓ and ↑ move the highlight, Home/End/Page Up/Page Down jump, Enter opens, Esc closes. The wheel scrolls a long folder.
- [ ] Closing: a click elsewhere (desktop, another window) closes it; a second click on the sensor closes it without reopening it; Alt+F4 closes it; a right-click on the bar closes it before the menu opens. No hints show while it is open.
- [ ] Right-click the sensor: *Open folder* opens it in Explorer. 125 % and 150 % display scale: the list is sharp, icons included.

**Folder Menu navigation** (since M3 S10b)

- [ ] A click on a folder row goes into it: the lists slide left over a quarter second, the header shows the subfolder's name and a Back arrow on its right. Back (or ←, or Backspace) slides back right to the folder as it was left, scrolled as before and with the subfolder marked. → on a marked folder goes in; Enter too. With *Animations* off (Settings ▸ Bar) the lists change at once.
- [ ] Type the first letters of a name: the list jumps to it; a pause of a second starts a new name.
- [ ] The wheel scrolls smoothly, by as many lines per notch as Settings ▸ Mouse says; a quick series of notches adds up.
- [ ] Above a bottom bar: the pointer turns into resize arrows over the right edge and the top edge; dragging them resizes the list (not below 130 px, not wider than 600 px or taller than 70 % of the screen), also when the pointer leaves the list while dragging. Reopen it: the new size stays, and Settings shows nothing new (the size is internal). A sensor at the screen's right edge, whose list lines up on the right: the left edge is the grip. A top bar: the bottom edge is the grip.

**Start Menu** (since M3 S11)

- [ ] *New sensor… ▸ Launchers ▸ Start Menu*: the Start Menu icon (21 px); hover: *Start Menu*. Click: the list shows the programs of both Start Menu folders as one: e.g. *Accessories* (or *Windows Accessories*), *Startup*, *Windows Tools* and the shortcuts, sorted as Explorer sorts. A folder that exists in both (e.g. *Accessories*, or a program's folder installed for you and for everyone) appears once and lists the shortcuts of both. CI renders it (`popup-start-menu.png`).
- [ ] A shortcut you copy into `%APPDATA%\Microsoft\Windows\Start Menu\Programs` under the name of an all-users one (e.g. a copy of *Edge*): it appears once, and opening it opens your copy.
- [ ] Navigation, keys, wheel and resizing work as in the Folder Menu; the header opens your own Start Menu folder of the level shown.
- [ ] Right-click: *Open folder* opens `%APPDATA%\Microsoft\Windows\Start Menu\Programs`; *Open (all users) folder* opens `C:\ProgramData\Microsoft\Windows\Start Menu\Programs`.
- [ ] *Title* → `Apps` changes the hint and the header; *Image file* replaces the icon.

**Eject USB** (since M3 S12)

- [ ] *New sensor… ▸ Launchers ▸ Eject USB*: the USB icon (30×21); hover: *Eject USB*, and with a stick plugged in *1 removable drive*. CI renders it in `bar-launchers*.png` and its list (`popup-eject.png`, made-up drives).
- [ ] Click without anything to eject: *Nothing to eject* above *Safely Remove Hardware…*. Plug a stick in while the list is open: its row appears within a second or two.
- [ ] Each row: the drive's icon as Explorer shows it, *STICK (E:)*, its size (*14.8 GB*), and *Eject*. A USB hard disk and a DVD with a disc are listed too; an empty DVD drive, an empty card reader and internal disks are not.
- [ ] *Eject*: the button says *Ejecting…*, then a toast *STICK (E:) can be safely removed.* and the row goes; the list stays open. With a file open on the stick: the toast says it is in use and the button is back.
- [ ] ↑/↓ mark the buttons and the footer, Enter acts; Esc closes. *Safely Remove Hardware…* opens Windows' dialog and closes the list.

**Drop onto the bar** (since M3 S13)

- [ ] Drag a file from Explorer over an empty part of the bar: the drag image stays, the pointer shows *Create link*, and the accent placer shows where it would go (left, centre or right zone by thirds, as when reordering). Drop: a Shortcut to the file appears there at once, without a Properties dialog; its icon is the file's.
- [ ] A folder makes a Folder Menu; a drive from *This PC* (`E:`) makes a Disk; a selection of a file, a folder and a drive makes three sensors in that order.
- [ ] Over a sensor: the placer is left of it on its left half and right of it on its right half; the drop goes there.
- [ ] Drag a link from Edge or Chrome (and Firefox) onto the bar: a web Shortcut with the globe icon whose hint is the link's text (*Open 'Example Domain'*); the click opens the page. Drag the address bar's padlock: the page's title.
- [ ] Text, a `mailto:` link or an image from a page: the no-drop pointer, nothing happens. Esc during the drag, or leaving the bar: the placer goes.
- [ ] The bar locked: drops still add sensors. A context menu open on the bar: nothing can be dropped until it closes.

**Drop onto sensors** (since M3 S14)

- [ ] Drag a file from the same drive over a Folder Menu: the sensor darkens, a hint above it says *Move 'report.pdf' to 'Downloads'*, the pointer shows *Move*. Drop: the file moves (the shell's progress shows for big files; Ctrl+Z in Explorer undoes it). From a USB stick: *Copy …*; three items: *Copy 3 items to 'Downloads'*.
- [ ] Hold Ctrl (copy), Shift (move) or Alt (shortcut) while dragging: the hint and pointer change at once. Alt: *Create shortcut to 'report.pdf' in 'Downloads'*, and the drop makes `report.pdf.lnk` there (a second one `report.pdf (2).lnk`).
- [ ] A file with the name of one already in the folder: Windows' conflict dialog (replace, skip, compare). Dragging the Folder Menu's own folder, or a file already in it, onto it: no-drop pointer, no hint.
- [ ] A link from Edge or Chrome onto a Folder Menu: *Create shortcut to 'https://…' in 'Downloads'*; the drop makes `Example Domain.url` (a page without a title: its address with `:/.?=&` as `-`), which opens the page.
- [ ] Files onto the Recycle Bin sensor: it darkens, *Delete 'report.pdf'* or *Delete 3 items*; the drop moves them to the Recycle Bin (asking first if the Recycle Bin's properties say so), and the count goes up at once.
- [ ] A link onto the Recycle Bin, or a drive onto a Folder Menu: the placer shows instead, and the drop makes a new sensor next to it. Onto any other sensor (a clock, the Start Menu): the placer, new sensors.

**Budgets** (M3 S15)

- [ ] Task Manager ▸ Details with a Folder Menu, the Start Menu, Eject USB and a few Shortcuts on the bar: *Memory* stays within 30 MB at idle, after opening each popup once, and after a drag from Explorer over the bar (the drag image helper and OLE load then). CI measured the first-run layout at 12.7 MB idle, 15.9 MB with Settings open, 14.4 MB after closing it, 0.039 % CPU (`b8d5d9e`).
- [ ] Scroll quickly through a folder of a few thousand files (`C:\Windows\System32`) in a Folder Menu: the bar's Shortcut and Disk icons never blink; the rows' icons fill in for what shows, not for every row passed.

**Close-out fixes** (M3 S15)

- [ ] Move a file from a folder on C: onto a Folder Menu for another folder on C:: it moves once, Explorer shows it gone from the source, and nothing is deleted twice (no error). The same onto the Recycle Bin: the file is in the Recycle Bin (restorable), not deleted for good.
- [ ] Drag a few hundred files from a network share over the bar and a Folder Menu: the bar and the drag stay smooth; dropping them on the bar's empty part asks the share only then.
- [ ] A Disk Meter or Disk on a mapped drive whose server is off, next to one on C:: C:'s keeps updating at its interval. Plug in a USB stick: Eject USB and a Disk on its letter show it within a couple of seconds, even when the stick is slow to mount.
- [ ] Eject a stick with a file open in Notepad: the toast names *notepad.exe* (or says it is in use); with a Command Prompt `cd` into it, it says it is in use without a program. A card reader in a USB-C dock or monitor: *Eject* takes out the card (or says it cannot), never the dock's keyboard and network.
- [ ] A laptop whose system disk is eMMC (often small cheap laptops): C: is not in Eject USB and its Disk has no *Eject*.
- [ ] Sleep and Hibernate from the Power sensor as a standard user work (no logged error *1314*). A config copied from a machine with hibernation, on one without: the Hibernate sensor's hint says it is not available here, and a click does nothing.
- [ ] Resize a Folder Menu that sits near the screen's right edge: widening it until it no longer fits beside the sensor stops at the screen edge (no jump to the other side); a list lined up on the right narrows from the left without jumping.
- [ ] A laptop on AC at 99–100 % with the unplug alert set: one toast per plugging in, not one each time the charge flaps.
- [ ] A Wi-Fi 6E network on channel 2 (6 GHz): the hint says *6 GHz, channel 2*.
- [ ] Open a Folder Menu, then hide the bar with the hotkey: the list goes too. Press the sensor of an open list and hold for a second before releasing: the list closes and stays closed. Close a list with Esc while the pointer is on its sensor: no hint until the pointer leaves and comes back.
- [ ] Eject USB with two sticks: mark the first with ↓, eject it with Enter; when it leaves, nothing is marked and Enter does nothing. Swap a stick for a bigger one in the same letter: the size updates.

## M4 — Classic, import, packaging

Exit criteria (SPEC §17): Appendix B imports exactly as specified; the installer works per user without admin rights.

The M0–M3 checks above still apply. CI renders the classic bar with every built-in skin (`bar-classic-*.png` in `infodock-screenshots`); the checks here cover real monitors and the live bar.

**Built-in skins** (since M4 S1)

- [ ] Until the Appearance page (M4 S4): quit Infodock, set `"theme": "classic"` and `"classic": { "skin": "gloss" }` under `settings` in `%APPDATA%\app.infodock.desktop\config.json`, start it again. For each of `flat`, `gloss`, `brushed`, `carbon` and `glass`: the bar shows the skin at 25 px, sharp at 100 % and 150 % (Gloss's dark bottom line one physical px at 100 %, two at 200 %; Carbon's diamonds larger at 150 %).
- [ ] `"background": { "color": "#3A6EA5" }`: the skin takes that colour. `"opacity": 50` as well: the skin fades, the sensors' text and icons do not.
- [ ] Glass on a top bar: its light line is at the bottom of the bar, towards the screen centre; on a bottom bar at the top. Gloss's dark line is at the bottom on either edge.
- [ ] *Highlight on hover* on: the skin brightens under the pointer, as the modern background does.
- [ ] An unknown skin (`"skin": "nope"`): Gloss. `"custom"` without an image: Flat.
- [ ] The classic bar on a 4K monitor at 150 %: memory within 30 MB at idle, idle CPU as with the modern bar (the skin is drawn once per size, not per frame).

**Skin packs** (since M4 S2)

- [ ] Make a pack: `%APPDATA%\app.infodock.desktop\skins\test\skin.json` with `{ "name": "Test", "image": "tile.png", "shadowBand": 9 }` and a PNG tile 34 px tall (or a legacy 25 + 9 px skin), then set `"skin": "test"` as above: the tile repeats across the bar from its left end, never stretched; at 150 %, and with *Height* 32, the bar part fills the bar's height. CI shows the same with a made-up pack (`bar-classic-pack*`).
- [ ] Add `imageTop` and `imageBottom` with different tiles: each edge uses its own. With `image` only, a top bar shows the same bar part as a bottom bar.
- [ ] `"background": { "color": "#A5703A" }`: the pack is tinted (its greys become shades of that colour); without a colour it keeps its own colours.
- [ ] Break the pack and start again, one at a time: delete `tile.png`; replace it with a 600 px wide image, a GIF renamed to `.png`, or a 2 MB file; make the tile 30 px tall; name `../x.png` in `skin.json`. Each time the bar shows Gloss and the log says why.
- [ ] `"skin": "custom"` and `"customSkin": "C:\\Users\\…\\wood.jpg"` (a JPEG within 512 px and 1 MB): the photo tiled across the bar at its height, tinted when a colour is set; a missing or larger file: Flat.

**Shadow band** (since M4 S3)

- [ ] The test pack above with `"shadowBand": 9` and a 34 px tile on a bottom bar: a soft shadow lies above the bar, over the desktop and windows. Maximise a window: it stops at the bar (the band reserves nothing) and the shadow lies over its bottom edge.
- [ ] Click, scroll and drag in the shadow: everything reaches the window below it (a scroll bar, Explorer's status bar); the pointer never shows the bar's.
- [ ] On a top bar the shadow is below the bar. A floating bar: the shadow is away from the nearer screen edge and moves with the bar while it is dragged.
- [ ] Start Infodock and change the edge: the shadow appears once the bar is in place, not while it slides. Hide the bar (hotkey or tray): the shadow goes too, and comes back with it.
- [ ] A full-screen video on the bar's monitor (*Hide on full screen* on): no shadow over the video. *Z-order* → *Bottom*: the shadow stays behind windows as the bar does; *Topmost*: above them.
- [ ] Switch to a built-in skin or the modern theme: the shadow goes. Two monitors at 100 % and 150 %: on the second the shadow is 1.5 times as tall.

**Appearance page** (since M4 S4)

- [ ] Settings ▸ Appearance lies between Bar and Sensors. *Theme* → *Classic*: the bar switches at once to the classic look (25 px, Arial, Gloss); the page shows the skin gallery. Back to *Modern*: the gallery goes.
- [ ] Each tile previews its skin as the bar draws it, sharp at 100 % and 150 % (open Settings on a 150 % monitor), as tall as the bar: *Height* 32 makes the previews taller. A click on a tile, or the arrow keys once the gallery has the focus, changes the bar's skin; the chosen tile is outlined. The test pack from S2 shows as a tile with its name.
- [ ] *Background colour* → *A colour of your choice*, `#3A6EA5`: the bar and every built-in preview take it; the pack's preview is tinted. *Use theme default* goes back to the theme's colour; *The theme's colour* hides the colour box and ignores the colour.
- [ ] *Background opacity* 50 %: the classic skin fades, its previews too, the sensors do not. *Use theme default* returns to 90 % (classic) or 80 % (modern).
- [ ] *Material* lists *Theme default (Acrylic)* (modern) or *(None)* (classic), then None, Acrylic and Mica, and each changes the bar. Turn off *Transparency effects* in Windows, close and reopen Settings: the list is disabled and says why.
- [ ] *Custom image*: its row shows the path box and *Browse…*; pick a JPEG within 512 px and 1 MB: the bar and the tile show it. A missing, larger or GIF file: the row says why and the bar is Flat.
- [ ] Set `"skin": "gone"` in the config (or delete the chosen pack's folder and reopen the page): a red line under *Skin* says Gloss is used, and no tile is outlined. A broken pack chosen: the line gives the reason.

**Skin pack management** (since M4 S5)

- [ ] With the classic theme, *Skin packs* lists the folders in the skins folder; *Open skins folder* opens `%APPDATA%\app.infodock.desktop\skins` in Explorer (created if missing).
- [ ] *Install from folder…* with the test pack's folder from S2 (rename the installed one first): it is listed, its tile appears in the gallery, and the folder in `skins` holds only `skin.json` and the tiles it names (no `readme.txt` copied along).
- [ ] Zip that folder in Explorer (*Send to ▸ Compressed folder*), and zip its contents without the folder: *Install from .zip…* installs both, named after the folder and after the `.zip` respectively. Choose the pack as the skin, edit a tile, zip and install it again: the bar shows the new tile at once.
- [ ] A `.zip` without `skin.json`, a renamed text file, a pack with a tile 30 px tall: the line under the buttons says why, in red, and nothing appears in `skins` (no folder starting with `.` is left either).
- [ ] Set `"skin": "nope"` (Gloss with the notice), then install a pack named `nope`: the bar and the gallery show it at once, without restarting.
- [ ] A broken folder in `skins` (e.g. `skin.json` naming a missing file) is listed with its reason in red; *Remove* moves it to the Recycle Bin (it can be restored from there).
- [ ] *Remove* the pack the bar shows: it goes to the Recycle Bin and the bar shows Gloss, which the gallery outlines.

**Colour scheme** (since M4 S6)

- [ ] Settings ▸ Appearance ▸ *Colour scheme* *Dark* with Windows in light mode: Settings turns dark at once, title bar included, and the modern bar is dark (`#1C1C1F`, white text). *Light* with Windows in dark mode: Settings and the bar turn light (`#F3F3F3`, dark text); the colour box shows `#F3F3F3` as the theme default.
- [ ] *Like Windows*: switch Windows between light and dark (Settings ▸ Personalisation ▸ Colours ▸ *Choose your mode*) with Infodock's Settings open: Settings and the bar follow within a second, without reopening.
- [ ] In light mode, click into the colour box and out again: the colour stays the theme's (the bar keeps its light look, *Use theme default* stays disabled). Set a sensor text colour or a background colour: the bar leaves the light look, as before.
- [ ] The classic theme with any colour scheme: the bar keeps its skin; only Settings changes.
- [ ] Close and reopen Settings with *Dark* chosen: it opens dark, title bar included.

**Import and export** (since M4 S7–S8)

- [ ] Right-click the bar ▸ *Tools*: *Import layout…* and *Export layout…* are enabled (and *Import legacy XML layout…* since S9), *Help* greyed out.
- [ ] *Export layout…*: the save picker suggests `Infodock layout` with the type *Infodock layout*; save it to the desktop: `Infodock layout.infodock.json` holds `schema`, `layout` and `state`, no settings. Export again with the type *Infodock settings and layout*: the file holds the whole config. A name typed without an extension gets `.infodock.json`; an existing file asks before it is replaced.
- [ ] While the picker is open, the bar's clock and graphs go on; a second *Export layout…* or *Import layout…* from the bar does nothing.
- [ ] Move a sensor, then *Import layout…* the first export: the picker lists `.infodock.json` files; the question shows *Replace the layout* and *Add to the layout* with the number of sensors. *Replace*: the bar shows the exported layout again, and a toast says "Layout imported" with the count. *Add to the layout*: the sensors are added at the end of their zones (with an Uptime sensor: its longest uptime comes along).
- [ ] Cancel at the picker or at the question: nothing changes.
- [ ] Edit the file: give a sensor `"opacity": "x"` and add an entry `{ "type": "mail" }` and an entry `3`. Import it: no toast; a dialog says how many were added and lists the entry without a type and the invalid value; the `mail` sensor shows as a placeholder.
- [ ] Import a text file renamed to `.infodock.json`, a file with `"schema": 99`, and `{ "layout": {} }`: a message says why (not a layout, a newer version, no sensors), and the layout is unchanged.
- [ ] With Settings open, the dialogs are modal to it (Settings cannot be clicked until they close); with Settings closed they appear in front of other windows.

**Legacy XML import** (since M4 S9–S11)

- [ ] *Tools ▸ Import legacy XML layout…* is enabled; its picker lists `.xml` files.
- [ ] Import `tests/fixtures/legacy/general.xml` with *Replace the layout*: on the left "Hello & welcome" in bold italic Tahoma at 25 px with a shadow, then a 20 px spacer; in the centre `C:\Pictures\logo.png` (the logo while the file is missing) tinted blue at half opacity, with the hint "My logo"; on the right the logo and "Plain". The report dialog lists *DockStandardMail* (not supported yet), *DockWeather* (unknown), the unknown property `blink` and the invalid `shadow` value.
- [ ] Export the layout afterwards: `legacy.unsupported` holds the mail sensor with its server and user, without its password.
- [ ] Import `tests/fixtures/legacy/settings-dump.txt` (a settings dump in Windows-1252 with the layout inside) with *Add to the layout*: "Café – open" in the centre and a spacer with a divider on the right are added; a toast says 2 sensors were added.
- [ ] A text file without `<sensors>`, and an XML file broken in the middle: a message says why, and the layout is unchanged.
- [ ] Import `tests/fixtures/legacy/system.xml` with *Replace the layout*: on the left the date as `Sat, 11` beside `oct` with the year below it, with a shadow and a Sunday-first calendar in its hint, a 12-hour clock with seconds (a chime on the half hour), and "Up 3d …" in 14 px with a shadow whose hint shows a longest uptime of 3 days, 4 hours, 12 minutes (unless this one is longer); in the centre the CPU meter (dynamic colour), the CPU graph 100 px wide on a dark grey background with dynamic columns, the dual CPU graph with a white grid and its lower half growing up, the memory meter in orange, the memory graph showing a percentage, the swap meter and the swap graph in silver; on the right the D: disk meter showing the free bytes (`n/a` without a D: drive) and the network graph 150 px wide on Automatic. The report lists the swap graph's `display-mode` and the Realtek adapter.
- [ ] Import `tests/fixtures/legacy/example.xml` (SPEC Appendix B) with *Replace the layout*: on the left the battery, a spacer with a divider, the C: disk meter titled "System", the Recycle Bin (its icons missing: the built-in ones, at 77 % opacity) and a "Documents" shortcut to `D:\Documents`; on the right the memory graph in silver, the dual CPU graph in grey and silver, the uptime without a prefix in 11 px Arial, and the date as the day alone with the year over the month's number. The report lists only *DockRSSSensor* (not supported yet); an export holds it in `legacy.unsupported` with its title and address.
- [ ] Import `tests/fixtures/legacy/launchers.xml`: the Wi-Fi sensor "Home" showing dBm, the E: disk tinted light blue, a "Downloads" Folder Menu whose popup opens 300 × 450 px, the Start Menu whose popup opens 500 px tall, Power shutting down without asking and Power locking with the question, and Eject USB at 70 %. The report lists Power's `path` once (the second Power's is empty).
- [ ] Edit the file's `net-adapter` to this computer's adapter as Device Manager names it (or its connection's name, e.g. `Wi-Fi`) and import again: the network graph shows that connection, and the report no longer lists it.
