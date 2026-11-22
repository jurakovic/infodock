# Measures Infodock's budgets: the private working set (Task Manager's
# "Memory" column) of infodock.exe and all its child processes, at idle, with Settings open,
# and after Settings was closed; its CPU use at idle over 60 s, as a share of all processors
# (Task Manager's "CPU" column); and the time from process creation to the bar's first frame,
# which the app logs, of the first run (cold) and the measured run (warm). Adds the times the
# budget tests recorded (`target/budgets/*.json`, `cargo test budget_ -- --ignored`) and the
# memory the app logged itself (at the bar's first frame, 10 s after Settings opened and
# closed). Also saves screenshots and prints the app's log. Writes summary tables to the job
# summary.
#
# The first run opens the welcome page: a run before the measured one measures it
# (reported, not enforced) and closes it, which ends the first run, so the measured run starts
# with the bar alone. Restart Manager closes the first run, as an installer that uses it would;
# the measured run is quit with taskkill without /f. Each must quit within 10 s: one that has to
# be killed lists its windows and is a failure.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $OutDir,
    # The folder of the budget tests' records.
    [string] $Measured,
    [int] $BudgetMb = 30,
    [double] $CpuBudgetPercent = 0.5,
    [int] $CpuSeconds = 60,
    [int] $ColdStartBudgetMs = 500,
    [int] $WarmStartBudgetMs = 300,
    # Fail the step when a measurement without Settings is over budget or missing, or when the
    # app had to be killed.
    [switch] $Enforce
)
# The records the budget tests write, in the order INTERNALS.md, Budgets lists them.
$Recorded = @('redraw', 'sampler', 'hint', 'folder-menu', 'folder-menu-described')
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class User32 {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
}
'@
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class RestartManager {
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmStartSession(out uint session, int flags, StringBuilder key);
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmRegisterResources(uint session, uint files, string[] names, uint apps, IntPtr processes, uint services, string[] serviceNames);
    [DllImport("rstrtmgr.dll")]
    static extern int RmShutdown(uint session, uint flags, IntPtr progress);
    [DllImport("rstrtmgr.dll")]
    static extern int RmEndSession(uint session);

    // Asks the apps that have `file` open to close, without forcing them, as an installer that
    // replaces the file does; 0 once they closed, else a Win32 error.
    public static int Shutdown(string file) {
        uint session;
        var key = new StringBuilder(33); // CCH_RM_SESSION_KEY + 1
        int error = RmStartSession(out session, 0, key);
        if (error != 0) { return error; }
        try {
            error = RmRegisterResources(session, 1, new[] { file }, 0, IntPtr.Zero, 0, null);
            return error != 0 ? error : RmShutdown(session, 0, IntPtr.Zero);
        } finally {
            RmEndSession(session);
        }
    }
}
public static class AppWindows {
    delegate bool EnumProc(IntPtr window, IntPtr data);
    [DllImport("user32.dll")]
    static extern bool EnumWindows(EnumProc proc, IntPtr data);
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassNameW(IntPtr window, StringBuilder name, int size);
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    static extern IntPtr GetWindow(IntPtr window, uint relation);

    // The top-level windows of process `id`, the top of the z-order first.
    public static string[] Of(uint id) {
        var found = new List<string>();
        EnumWindows((window, data) => {
            uint process;
            uint thread = GetWindowThreadProcessId(window, out process);
            if (process == id) {
                var name = new StringBuilder(256);
                GetClassNameW(window, name, name.Capacity);
                found.Add(string.Format("{0} (thread {1}{2}{3})", name, thread,
                    IsWindowVisible(window) ? ", visible" : "",
                    GetWindow(window, 4 /* GW_OWNER */) != IntPtr.Zero ? ", owned" : ""));
            }
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
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

$results = [System.Collections.Generic.List[object]]::new()

# Measures the stage $Label; one measured with -ReportOnly is listed but never fails the step.
function Measure-Tree([string] $Label, [int] $RootId, [switch] $ReportOnly) {
    $rows = foreach ($p in Get-ProcessTree $RootId) {
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess = $($p.ProcessId)"
        if ($perf) {
            [pscustomobject]@{ Process = $p.Name; Id = $p.ProcessId; PrivateMb = [math]::Round($perf.WorkingSetPrivate / 1MB, 1) }
        }
    }
    $total = [math]::Round((($rows | Measure-Object PrivateMb -Sum).Sum), 1)
    Write-Host "== ${Label}: $total MB in $(@($rows).Count) process(es)"
    $rows | Format-Table -AutoSize | Out-String | Write-Host
    $results.Add([pscustomobject]@{ Stage = $Label; TotalMb = $total; Processes = @($rows).Count; ReportOnly = [bool]$ReportOnly })
}

function Save-Screenshot([string] $Path) {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
}

# Closes the Settings window as its close button does (SC_CLOSE, which sends it WM_CLOSE; a
# WM_CLOSE posted to any of the app's windows quits the app); $false if it is not open.
function Close-Settings {
    # [NullString]: PowerShell would pass $null to a string parameter as "".
    $settings = [User32]::FindWindowW([NullString]::Value, 'Infodock Settings')
    if ($settings -eq [IntPtr]::Zero) { return $false }
    [User32]::PostMessageW($settings, 0x0112, [IntPtr]0xF060, [IntPtr]::Zero) | Out-Null # WM_SYSCOMMAND, SC_CLOSE
    return $true
}

# How each run ended: whether it quit when asked.
$quits = [System.Collections.Generic.List[object]]::new()

# Waits up to 10 s for the app to quit; kills it after listing its windows if it did not.
function Wait-Quit($Process, [string] $Run) {
    $quit = $Process.WaitForExit(10000)
    if (-not $quit) {
        Write-Warning "Infodock did not quit within 10 s ($Run); killing it. Its windows, the top of the z-order first:"
        [AppWindows]::Of([uint32]$Process.Id) | ForEach-Object { Write-Host "  $_" }
        & taskkill.exe /f /t /pid $Process.Id | Out-Null
    }
    $quits.Add([pscustomobject]@{ Run = $Run; Quit = $quit })
}

# Graceful quit: taskkill without /f, which posts WM_CLOSE to one of the app's windows.
function Stop-Gracefully($Process, [string] $Run) {
    if ($Process.HasExited) { return }
    & taskkill.exe /pid $Process.Id | Out-Null
    Wait-Quit $Process $Run
}

# Restart Manager asks the app to close (WM_QUERYENDSESSION and WM_ENDSESSION, as for a
# sign-out), as an installer that replaces the exe does.
function Stop-WithRestartManager($Process, [string] $Run) {
    if ($Process.HasExited) { return }
    $code = [RestartManager]::Shutdown((Resolve-Path $Exe).Path)
    if ($code -ne 0) {
        Write-Warning "Restart Manager could not close Infodock (error $code)"
        $Run = "$Run (Restart Manager error $code)"
    }
    Wait-Quit $Process $Run
}

$LogDir = Join-Path $env:LOCALAPPDATA 'app.infodock.desktop\logs'

function Show-Log {
    if (Test-Path $LogDir) {
        Get-ChildItem $LogDir -Filter 'infodock*.log' | ForEach-Object {
            Write-Host "== $($_.Name)"
            Get-Content $_.FullName | Write-Host
            Copy-Item $_.FullName $OutDir
        }
    }
}

# The starts the app logged, oldest first (daily log files sort by date): the time to the
# bar's first frame and when the start-up's steps were done.
function Get-StartTimes {
    if (-not (Test-Path $LogDir)) { return @() }
    $lines = Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content
    @($lines | Select-String -Pattern 'bar visible (\d+) ms after the process started(?: \((.*)\))?' |
        ForEach-Object {
            [pscustomobject]@{ Ms = [int]$_.Matches[0].Groups[1].Value; Steps = $_.Matches[0].Groups[2].Value }
        })
}

# The memory the app logged, oldest first: at each start's first frame (which begins a run)
# and 10 s after Settings opened and closed.
function Get-LoggedMemory {
    if (-not (Test-Path $LogDir)) { return @() }
    $lines = Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content
    $run = 0
    $pattern = '\bmemory ((?:at|\d+ s after) [^:]+): (.+)$'
    $readings = foreach ($line in ($lines | Select-String -Pattern $pattern)) {
        $when = $line.Matches[0].Groups[1].Value
        if ($when.StartsWith('at ')) { $run++ }
        [pscustomobject]@{ Run = $run; When = $when; Reading = $line.Matches[0].Groups[2].Value }
    }
    @($readings)
}

# The start a run logged: the one more than before it; $null if it logged none.
function Get-NewStartTime([object[]] $Before) {
    $after = @(Get-StartTimes)
    if ($after.Count -gt $Before.Count) { $after[-1] } else { $null }
}

$startTimes = @(Get-StartTimes)

# The first run: the welcome page, measured and closed. The app logs its memory 10 s after
# Settings closed, so the run lasts until then.
$first = Start-Process -FilePath $Exe -PassThru
try {
    Start-Sleep -Seconds 10
    if ($first.HasExited) { throw "Infodock exited during its first run (code $($first.ExitCode))" }
    Measure-Tree 'First run, welcome page open' $first.Id -ReportOnly
    Save-Screenshot (Join-Path $OutDir 'welcome.png')
    if (-not (Close-Settings)) { Write-Warning 'The welcome page did not open on the first run' }
    Start-Sleep -Seconds 12
} finally {
    Stop-WithRestartManager $first 'the first run, by Restart Manager'
}
$cold = Get-NewStartTime $startTimes
$startTimes = @(Get-StartTimes)

$app = Start-Process -FilePath $Exe -PassThru
try {
    Start-Sleep -Seconds 10
    if ($app.HasExited) { throw "Infodock exited during start-up (code $($app.ExitCode))" }
    Measure-Tree 'Idle' $app.Id
    Save-Screenshot (Join-Path $OutDir 'bar.png')

    # Idle CPU: processor time used over the window, of all processors' time.
    $before = (Get-Process -Id $app.Id).TotalProcessorTime
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $CpuSeconds
    $used = (Get-Process -Id $app.Id).TotalProcessorTime - $before
    $capacity = $clock.Elapsed.TotalSeconds * [Environment]::ProcessorCount
    $cpuPercent = [math]::Round(100 * $used.TotalSeconds / $capacity, 3)
    Write-Host "== Idle CPU: $cpuPercent % over $CpuSeconds s ($([math]::Round($used.TotalMilliseconds)) ms on $([Environment]::ProcessorCount) processors)"

    # A second launch opens Settings in the running instance.
    $second = Start-Process -FilePath $Exe -PassThru
    if (-not $second.WaitForExit(15000)) { Write-Warning 'The second instance did not exit' }
    Start-Sleep -Seconds 10
    Measure-Tree 'Settings open' $app.Id -ReportOnly
    Save-Screenshot (Join-Path $OutDir 'settings.png')

    if (-not (Close-Settings)) { Write-Warning 'Settings window not found' }
    Start-Sleep -Seconds 10
    if ($app.HasExited) { throw 'Infodock quit when Settings was closed' }
    Measure-Tree 'Settings closed' $app.Id
    # The app's own reading 10 s after Settings closed may follow the measurement.
    Start-Sleep -Seconds 2
} finally {
    Stop-Gracefully $app 'the measured run, by taskkill'
    Show-Log
}
$warm = Get-NewStartTime $startTimes

# Times: the starts, then the budget tests' records.
$times = [System.Collections.Generic.List[object]]::new()
$times.Add([pscustomobject]@{ Label = 'Cold start to bar visible'; Ms = $cold.Ms; BudgetMs = $ColdStartBudgetMs })
$times.Add([pscustomobject]@{ Label = 'Warm start to bar visible'; Ms = $warm.Ms; BudgetMs = $WarmStartBudgetMs })
foreach ($id in $Recorded) {
    $file = if ($Measured) { Join-Path $Measured "$id.json" }
    if ($file -and (Test-Path $file)) {
        $record = Get-Content $file -Raw | ConvertFrom-Json
        $times.Add([pscustomobject]@{ Label = $record.label; Ms = [double]$record.measuredMs; BudgetMs = [double]$record.budgetMs })
    } else {
        $times.Add([pscustomobject]@{ Label = "$id (budget test)"; Ms = $null; BudgetMs = $null })
    }
}

$summary = @("### Memory (private working set, budget $BudgetMb MB)", '', '| Stage | Total MB | Processes |', '|---|---:|---:|')
$summary += $results | ForEach-Object {
    $stage = if ($_.ReportOnly) { "$($_.Stage) (report only)" } else { $_.Stage }
    "| $stage | $($_.TotalMb) | $($_.Processes) |"
}
$logged = @(Get-LoggedMemory)
if ($logged) {
    $summary += @('', '### Memory the app logged', '', 'Run 1 is the first run (the welcome page), run 2 the measured run.', '', '| Run | When | Reading |', '|---:|---|---|')
    $summary += $logged | ForEach-Object { "| $($_.Run) | $($_.When) | $($_.Reading) |" }
}
$summary += @('', "### Idle CPU (budget $CpuBudgetPercent %)", '', "$cpuPercent % of $([Environment]::ProcessorCount) processors over $CpuSeconds s")
$summary += @('', '### Times', '', '| Measure | Measured | Budget |', '|---|---:|---:|')
$summary += $times | ForEach-Object {
    $measured = if ($null -eq $_.Ms) { 'not measured' } elseif ($_.Ms -gt $_.BudgetMs) { "**$($_.Ms) ms** (over)" } else { "$($_.Ms) ms" }
    $budget = if ($null -eq $_.BudgetMs) { '' } else { "$($_.BudgetMs) ms" }
    "| $($_.Label) | $measured | $budget |"
}
if ($cold.Steps) { $summary += @('', "Cold start, ms after the process started: $($cold.Steps)") }
if ($warm.Steps) { $summary += @('', "Warm start, ms after the process started: $($warm.Steps)") }
$summary += @('', '### Quitting', '', '| Run | Result |', '|---|---|')
$summary += $quits | ForEach-Object { "| $($_.Run) | $(if ($_.Quit) { 'quit within 10 s' } else { '**killed** after 10 s' }) |" }
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host

$problems = @()
$over = $results | Where-Object { -not $_.ReportOnly -and $_.TotalMb -gt $BudgetMb }
if ($over) {
    $problems += "over the $BudgetMb MB budget: " + (($over | ForEach-Object { "$($_.Stage) $($_.TotalMb) MB" }) -join ', ')
}
if ($cpuPercent -gt $CpuBudgetPercent) {
    $problems += "idle CPU $cpuPercent % is over the $CpuBudgetPercent % budget"
}
foreach ($time in $times) {
    if ($null -eq $time.Ms) {
        $problems += "$($time.Label) was not measured"
    } elseif ($time.Ms -gt $time.BudgetMs) {
        $problems += "$($time.Label) $($time.Ms) ms is over the $($time.BudgetMs) ms budget"
    }
}
$killed = @($quits | Where-Object { -not $_.Quit } | ForEach-Object { $_.Run })
if ($killed) {
    $problems += 'Infodock had to be killed: ' + ($killed -join '; ')
}
if ($problems) {
    $message = 'Checks failed: ' + ($problems -join '; ')
    if ($Enforce) { throw $message } else { Write-Warning $message }
}
