# Measures the WinUI spike of the Settings window (settings-winui/, plan D88) as
# measure-memory.ps1 measures the app: for each page and colour scheme, the private working set
# (Task Manager's "Memory" column) of the process and any children 10 s after start, and the
# time from the process's creation to its first frame, which it logs; a screenshot of the
# window; the size of the published folder. Writes summary tables to the job summary.
param(
    [Parameter(Mandatory)] [string] $Dir,
    [Parameter(Mandatory)] [string] $OutDir
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Exe = Join-Path $Dir 'infodock-settings.exe'
$Log = Join-Path $OutDir 'infodock-settings.log'
# A config that does not exist: the defaults, as on a first run.
$Config = Join-Path $OutDir 'none.json'

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("dwmapi.dll")]
    public static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out Rect rect, int size);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
}
'@

function Get-ProcessTree([int] $RootId) {
    $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($RootId)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($child in $all | Where-Object { $_.ParentProcessId -eq $ids[$i] }) {
            $ids.Add([int]$child.ProcessId)
        }
    }
    $all | Where-Object { $ids.Contains([int]$_.ProcessId) }
}

function Measure-Private([int] $RootId) {
    $bytes = 0.0
    $count = 0
    foreach ($p in Get-ProcessTree $RootId) {
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess = $($p.ProcessId)"
        if ($perf) {
            $bytes += $perf.WorkingSetPrivate
            $count++
        }
    }
    [pscustomobject]@{ Mb = [math]::Round($bytes / 1MB, 1); Processes = $count }
}

# The window once it is shown.
function Wait-Window($Process) {
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt 30) {
        $Process.Refresh()
        if ($Process.HasExited) { throw "The spike exited during start-up (code $($Process.ExitCode))" }
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) { return $Process.MainWindowHandle }
        Start-Sleep -Milliseconds 100
    }
    throw 'The spike showed no window within 30 s'
}

function Save-Window([IntPtr] $Window, [string] $Path) {
    [Win]::SetForegroundWindow($Window) | Out-Null
    Start-Sleep -Milliseconds 500
    $rect = New-Object 'Win+Rect'
    # DWMWA_EXTENDED_FRAME_BOUNDS: the window as drawn, without the invisible resize borders.
    [Win]::DwmGetWindowAttribute($Window, 9, [ref] $rect, 16) | Out-Null
    $bitmap = New-Object System.Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
}

# The starts logged so far: the time to the first frame and when the steps were done.
function Get-StartTimes {
    if (-not (Test-Path $Log)) { return @() }
    @(Get-Content $Log | Select-String -Pattern 'settings visible (\d+) ms after the process started(?: \((.*)\))?' |
        ForEach-Object {
            [pscustomobject]@{ Ms = [int]$_.Matches[0].Groups[1].Value; Steps = $_.Matches[0].Groups[2].Value }
        })
}

$runs = @(
    @{ Name = 'general-light'; Page = 'general'; Theme = 'light' },
    @{ Name = 'general-dark'; Page = 'general'; Theme = 'dark' },
    @{ Name = 'appearance-light'; Page = 'appearance'; Theme = 'light' },
    @{ Name = 'appearance-dark'; Page = 'appearance'; Theme = 'dark' }
)
$results = [System.Collections.Generic.List[object]]::new()
foreach ($run in $runs) {
    $before = @(Get-StartTimes).Count
    $app = Start-Process -FilePath $Exe -PassThru -ArgumentList @(
        '--page', $run.Page, '--theme', $run.Theme, '--config', $Config, '--log', $Log)
    try {
        $window = Wait-Window $app
        Start-Sleep -Seconds 10
        $memory = Measure-Private $app.Id
        Write-Host "== $($run.Name): $($memory.Mb) MB in $($memory.Processes) process(es)"
        Save-Window $window (Join-Path $OutDir "$($run.Name).png")
        [Win]::PostMessageW($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null # WM_CLOSE
        if (-not $app.WaitForExit(10000)) { Write-Warning "$($run.Name) did not exit within 10 s of WM_CLOSE" }
    } finally {
        if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
    }
    $times = @(Get-StartTimes)
    $start = if ($times.Count -gt $before) { $times[-1] } else { $null }
    $results.Add([pscustomobject]@{ Run = $run.Name; Mb = $memory.Mb; Processes = $memory.Processes; Start = $start })
}
if (Test-Path $Log) { Get-Content $Log | Write-Host }

# What would ship: the folder (symbols already dropped), its exe, and the folder zipped.
$files = @(Get-ChildItem $Dir -Recurse -File)
$folderMb = [math]::Round(($files | Measure-Object Length -Sum).Sum / 1MB, 1)
$exeMb = [math]::Round((Get-Item $Exe).Length / 1MB, 1)
$zip = Join-Path ([System.IO.Path]::GetTempPath()) 'infodock-settings-winui.zip'
Compress-Archive -Path (Join-Path $Dir '*') -DestinationPath $zip -Force
$zipMb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Remove-Item $zip

$summary = @('### WinUI Settings spike', '', '| Run | Private working set | Processes | First frame after the process started |', '|---|---:|---:|---:|')
$summary += $results | ForEach-Object {
    $ms = if ($_.Start) { "$($_.Start.Ms) ms" } else { 'not logged' }
    "| $($_.Run) | $($_.Mb) MB | $($_.Processes) | $ms |"
}
$summary += @('', 'Steps, ms after the process started:', '')
$summary += $results | Where-Object { $_.Start } | ForEach-Object { "* $($_.Run): $($_.Start.Steps)" }
$summary += @('', "Published folder: $folderMb MB in $($files.Count) files (exe $exeMb MB), $zipMb MB zipped.", '', 'Largest files:', '')
$summary += $files | Sort-Object Length -Descending | Select-Object -First 10 | ForEach-Object {
    "* $($_.FullName.Substring($Dir.Length).TrimStart('\', '/')): $([math]::Round($_.Length / 1MB, 1)) MB"
}
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host
