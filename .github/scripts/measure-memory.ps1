# Measures Infodock's budgets (SPEC §14.1, §16): the private working set (Task Manager's
# "Memory" column) of infodock.exe and all its child processes, at idle, with Settings open,
# and after Settings was closed; its CPU use at idle over 60 s, as a share of all processors
# (Task Manager's "CPU" column); and the time from process creation to the bar's first frame,
# which the app logs, of the first run (cold) and the measured run (warm). Adds the times the
# budget tests recorded (`target/budgets/*.json`, `cargo test budget_ -- --ignored`). Also
# saves screenshots, quits the app gracefully and prints its log. Writes summary tables to the
# job summary.
#
# The first run opens the welcome page (SPEC §11.5): a run before the measured one closes it,
# which ends the first run, so the measured run starts with the bar alone.
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
    # Fail the step when a measurement without Settings is over budget or missing.
    [switch] $Enforce
)
# The records the budget tests write, in the order of SPEC §14.1.
$Recorded = @('redraw', 'sampler', 'hint', 'folder-menu')
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

function Measure-Tree([string] $Label, [int] $RootId) {
    $rows = foreach ($p in Get-ProcessTree $RootId) {
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess = $($p.ProcessId)"
        if ($perf) {
            [pscustomobject]@{ Process = $p.Name; Id = $p.ProcessId; PrivateMb = [math]::Round($perf.WorkingSetPrivate / 1MB, 1) }
        }
    }
    $total = [math]::Round((($rows | Measure-Object PrivateMb -Sum).Sum), 1)
    Write-Host "== ${Label}: $total MB in $(@($rows).Count) process(es)"
    $rows | Format-Table -AutoSize | Out-String | Write-Host
    $results.Add([pscustomobject]@{ Stage = $Label; TotalMb = $total; Processes = @($rows).Count })
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

# Closes the Settings window as its close button does; $false if it is not open.
function Close-Settings {
    # [NullString]: PowerShell would pass $null to a string parameter as "".
    $settings = [User32]::FindWindowW([NullString]::Value, 'Infodock Settings')
    if ($settings -eq [IntPtr]::Zero) { return $false }
    [User32]::PostMessageW($settings, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null # WM_CLOSE
    return $true
}

# Graceful quit: WM_CLOSE to the app's windows, as Restart Manager or a plain taskkill does.
function Stop-Gracefully($Process) {
    if (-not $Process.HasExited) {
        & taskkill.exe /pid $Process.Id | Out-Null
        if (-not $Process.WaitForExit(10000)) {
            Write-Warning 'Infodock did not quit within 10 s; killing it'
            & taskkill.exe /f /t /pid $Process.Id | Out-Null
        }
    }
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

# The start times the app logged, oldest first (daily log files sort by date).
function Get-StartTimes {
    if (-not (Test-Path $LogDir)) { return @() }
    $lines = Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content
    @($lines | Select-String -Pattern 'bar visible (\d+) ms after the process started' |
        ForEach-Object { [int]$_.Matches[0].Groups[1].Value })
}

# The start time a run logged: the one more than before it; $null if it logged none.
function Get-NewStartTime([object[]] $Before) {
    $after = @(Get-StartTimes)
    if ($after.Count -gt $Before.Count) { $after[-1] } else { $null }
}

$startTimes = @(Get-StartTimes)

# The first run: the welcome page, closed.
$first = Start-Process -FilePath $Exe -PassThru
try {
    Start-Sleep -Seconds 10
    if ($first.HasExited) { throw "Infodock exited during its first run (code $($first.ExitCode))" }
    Save-Screenshot (Join-Path $OutDir 'welcome.png')
    if (-not (Close-Settings)) { Write-Warning 'The welcome page did not open on the first run' }
    Start-Sleep -Seconds 3
} finally {
    Stop-Gracefully $first
}
$coldMs = Get-NewStartTime $startTimes
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

    # A second launch opens Settings in the running instance (SPEC §6.5).
    $second = Start-Process -FilePath $Exe -PassThru
    if (-not $second.WaitForExit(15000)) { Write-Warning 'The second instance did not exit' }
    Start-Sleep -Seconds 10
    Measure-Tree 'Settings open' $app.Id
    Save-Screenshot (Join-Path $OutDir 'settings.png')

    if (-not (Close-Settings)) { Write-Warning 'Settings window not found' }
    Start-Sleep -Seconds 10
    if ($app.HasExited) { throw 'Infodock quit when Settings was closed' }
    Measure-Tree 'Settings closed' $app.Id
} finally {
    Stop-Gracefully $app
    Show-Log
}
$warmMs = Get-NewStartTime $startTimes

# Times (SPEC §14.1): the starts, then the budget tests' records.
$times = [System.Collections.Generic.List[object]]::new()
$times.Add([pscustomobject]@{ Label = 'Cold start to bar visible'; Ms = $coldMs; BudgetMs = $ColdStartBudgetMs })
$times.Add([pscustomobject]@{ Label = 'Warm start to bar visible'; Ms = $warmMs; BudgetMs = $WarmStartBudgetMs })
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
$summary += $results | ForEach-Object { "| $($_.Stage) | $($_.TotalMb) | $($_.Processes) |" }
$summary += @('', "### Idle CPU (budget $CpuBudgetPercent %)", '', "$cpuPercent % of $([Environment]::ProcessorCount) processors over $CpuSeconds s")
$summary += @('', '### Times', '', '| Measure | Measured | Budget |', '|---|---:|---:|')
$summary += $times | ForEach-Object {
    $measured = if ($null -eq $_.Ms) { 'not measured' } elseif ($_.Ms -gt $_.BudgetMs) { "**$($_.Ms) ms** (over)" } else { "$($_.Ms) ms" }
    $budget = if ($null -eq $_.BudgetMs) { '' } else { "$($_.BudgetMs) ms" }
    "| $($_.Label) | $measured | $budget |"
}
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host

$problems = @()
$over = $results | Where-Object { $_.Stage -ne 'Settings open' -and $_.TotalMb -gt $BudgetMb }
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
if ($problems) {
    $message = 'Budgets missed: ' + ($problems -join '; ')
    if ($Enforce) { throw $message } else { Write-Warning $message }
}
