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

The M0–M2 checks above still apply. CI renders the launchers (`bar-launchers*.png` in `infodock-screenshots`) with this machine's shell icons; the checks here cover clicking, the shell and real devices.

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
