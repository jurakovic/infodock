# Settings in WinUI 3 (spike)

A comparison spike for plan D88: two pages of Infodock's Settings window built with WinUI 3, so
the look, the feel and the cost can be set against the Slint window. It lives on its own branch
and is not part of the app.

## What it is

- WinUI 3 (Windows App SDK 1.8) in C# on .NET 10, unpackaged, self-contained and compiled
  ahead of time (Native AOT): the folder runs on Windows 10 1809 or later without installing
  .NET or the Windows App SDK runtime.
- The real controls: `NavigationView`, the Community Toolkit's `SettingsCard` (the card
  PowerToys and Dev Home use to look like Windows 11 Settings), `ToggleSwitch`, `ComboBox`,
  `Slider`, `ColorPicker`, Mica behind the window, the extended title bar.
- The General and Appearance pages, row for row as in `ui/settings.slint`, with the strings
  from `i18n/en.json`. The other six pages say they are not part of the spike; Appearance
  leaves out the classic theme's skin gallery and packs, whose previews the bar draws.
- It reads your settings from `%APPDATA%\app.infodock.desktop\config.json`, read-only, and
  saves nothing. It does not talk to a running Infodock. Controls move, the colour scheme
  switches the window live, the hotkey button records a key, the colour opens WinUI's picker.
- The window opens at 760 × 560, as the Slint window does.

## Running it

Download the `infodock-settings-winui` artifact of the *WinUI spike* workflow, unzip it and run
`infodock-settings.exe`. Options:

| Option | |
|---|---|
| `--page appearance` | open a page (`general`, `appearance`, …) |
| `--theme light` / `--theme dark` | instead of the configured colour scheme |
| `--config path` | another config.json |
| `--log path` | where the start-up line goes (next to the exe by default) |

Each start appends `settings visible N ms after the process started (…)` to the log, with when
each step was done, the form the bar uses. Task Manager's *Memory* column is the private
working set CI measures.

## CI

`.github/workflows/winui-spike.yml` publishes it on `windows-latest`, then
`.github/scripts/measure-winui.ps1` opens each page in light and dark, measures the private
working set 10 s after start and the time to the first frame, takes a screenshot, and lists
the size of the folder. The job summary has the tables; the `infodock-winui-check` artifact
the screenshots and the log.

## Building

On Windows with the .NET 10 SDK and the C++ build tools (for Native AOT):

```
dotnet publish settings-winui/InfodockSettings.csproj -c Release -o out
```

The XAML compiler runs only on Windows, so this does not build in a Linux session.
