# Checks that Infodock never keeps Windows from locking a drive, as Windows locks a volume before
# it removes or ejects it, with the release exe on the runner. A 64 MB VHD is attached as a fixed
# NTFS drive, with an item in its Recycle Bin. For each layout below, the app starts, and the
# script locks and unlocks the drive (FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME) as fast as it can for
# a minute: a reading that has the drive open when a lock comes makes the lock fail, as it makes a
# removal fail, and readings hold a drive only for a moment, so only a tight loop catches one.
# Without the app the drive locks, and with a file held open on it the lock fails, so the loop can
# see a holder. The app's log must also say, under each layout, that it watched the drive again
# after an unlock, and its last line of letting go of the drive or watching it again must be a
# watch-again: a drive let go of and never watched again is read no more, so every lock would
# take while its Recycle Bin goes uncounted, its space reads as absent and the drive list keeps
# its last listing. The app's log lines about drives are kept. Runs after the Show desktop check
# and before the installer check, which removes the config; the config found is put back at the
# end.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $OutDir,
    # Fail the step when a check fails; without it they are warnings.
    [switch] $Enforce,
    # How long each layout's loop runs.
    [int] $Seconds = 60
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class DriveLock {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DeviceIoControl(SafeFileHandle device, uint code, IntPtr input, uint inputSize, IntPtr output, uint outputSize, out uint returned, IntPtr overlapped);

    const uint GENERIC_READ = 0x80000000, GENERIC_WRITE = 0x40000000;
    const uint FILE_SHARE_READ = 1, FILE_SHARE_WRITE = 2, OPEN_EXISTING = 3;
    const uint FSCTL_LOCK_VOLUME = 0x00090018, FSCTL_UNLOCK_VOLUME = 0x0009001C;

    // Locks drive `letter` and unlocks it again: 0 when the lock took, else the Windows error.
    public static int LockOnce(string letter) {
        using (var volume = CreateFileW(@"\\.\" + letter + ":", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero)) {
            if (volume.IsInvalid) return Marshal.GetLastWin32Error();
            uint returned;
            if (!DeviceIoControl(volume, FSCTL_LOCK_VOLUME, IntPtr.Zero, 0, IntPtr.Zero, 0, out returned, IntPtr.Zero)) {
                return Marshal.GetLastWin32Error();
            }
            DeviceIoControl(volume, FSCTL_UNLOCK_VOLUME, IntPtr.Zero, 0, IntPtr.Zero, 0, out returned, IntPtr.Zero);
            return 0;
        }
    }

    public sealed class Result {
        public int Tries;
        public int Failures;
        // Failures by Windows error.
        public Dictionary<int, int> Errors = new Dictionary<int, int>();
        // When the first failures came, in seconds from the start of the loop.
        public List<double> FailedAt = new List<double>();
    }

    // Locks and unlocks drive `letter` as fast as it can for `seconds`.
    public static Result Loop(string letter, int seconds) {
        var result = new Result();
        var clock = Stopwatch.StartNew();
        while (clock.Elapsed.TotalSeconds < seconds) {
            var error = LockOnce(letter);
            result.Tries++;
            if (error == 0) continue;
            result.Failures++;
            int count;
            result.Errors.TryGetValue(error, out count);
            result.Errors[error] = count + 1;
            if (result.FailedAt.Count < 20) result.FailedAt.Add(Math.Round(clock.Elapsed.TotalSeconds, 2));
        }
        return result;
    }
}
'@
Add-Type -AssemblyName Microsoft.VisualBasic

$LogDir = Join-Path $env:LOCALAPPDATA 'app.infodock.desktop\logs'
function Get-LogLines {
    if (-not (Test-Path $LogDir)) { return @() }
    @(Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content)
}

function Get-ErrorText([int] $Code) {
    "$Code ($([System.ComponentModel.Win32Exception]::new($Code).Message))"
}

$results = [System.Collections.Generic.List[object]]::new()
$records = [System.Collections.Generic.List[string]]::new()
function Add-Result([string] $Name, [bool] $Passed, [string] $Detail = '') {
    $line = "$Name -> $(if ($Passed) { 'passed' } else { 'FAILED' })$(if ($Detail) { ": $Detail" })"
    Write-Host $line
    $records.Add($line)
    $results.Add([pscustomobject]@{ Name = $Name; Detail = $Detail; Passed = $Passed })
}

# Runs diskpart with `Commands`; its output goes to the record.
function Invoke-DiskPart([string[]] $Commands) {
    $script = Join-Path $OutDir 'diskpart.txt'
    $Commands | Set-Content -Encoding ascii $script
    $output = @(& diskpart.exe /s $script 2>&1)
    $output | Out-File -Append (Join-Path $OutDir 'diskpart.log')
    if ($LASTEXITCODE -ne 0) { throw "diskpart failed (exit code $LASTEXITCODE): $($output -join ' / ')" }
}

# Whether drive `Letter` locks within `Seconds`, trying every 200 ms.
function Wait-Lock([string] $Letter, [int] $Seconds) {
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    do {
        if ([DriveLock]::LockOnce($Letter) -eq 0) { return $true }
        Start-Sleep -Milliseconds 200
    } while ($clock.Elapsed.TotalSeconds -lt $Seconds)
    $false
}

# The app's lines among `Lines` that say it let go of drive `Letter` or watches it again.
function Select-Watch([object[]] $Lines, [string] $Letter) {
    @($Lines | Select-String -CaseSensitive -Pattern "let go of ${Letter}:|watching ${Letter}: again" | ForEach-Object Line)
}

# The loop's tries and failures, for the record.
function Format-Loop($Loop) {
    $text = "$($Loop.Tries) tries, $($Loop.Failures) failed"
    if ($Loop.Failures -gt 0) {
        $errors = @($Loop.Errors.GetEnumerator() | ForEach-Object { "$($_.Value) x $(Get-ErrorText $_.Key)" })
        $text += " (errors: $($errors -join ', '); first at $($Loop.FailedAt -join ', ') s)"
    }
    $text
}

$configDir = Join-Path $env:APPDATA 'app.infodock.desktop'
$config = Join-Path $configDir 'config.json'
$savedConfig = if (Test-Path $config) { Get-Content -Raw $config } else { $null }

# A config whose layout is `Sensors` (each a type and its props) on the left, default settings
# otherwise and the first run done.
function Write-Layout([object[]] $Sensors) {
    $index = 0
    $left = @($Sensors | ForEach-Object {
        $index++
        [ordered]@{ id = "s_drv0$index"; type = $_.Type; props = $_.Props }
    })
    $document = [ordered]@{
        schema = 1
        meta = [ordered]@{ firstRunDone = $true }
        layout = [ordered]@{ left = $left; center = @(); right = @() }
    }
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null
    $document | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8NoBOM $config
}

$recycleBin = @{ Type = 'recycle-bin'; Props = @{ interval = 5 } }
$disk = @{ Type = 'disk'; Props = @{ drive = 'C:\' } }
$ejectUsb = @{ Type = 'eject-usb'; Props = @{} }
$diskMeter = @{ Type = 'disk-meter'; Props = @{ drive = 'C:\' } }
$layouts = @(
    @{ Name = 'a Recycle Bin read every 5 s'; Sensors = @($recycleBin) },
    @{ Name = 'a Disk for C: and Eject USB'; Sensors = @($disk, $ejectUsb) },
    @{ Name = 'a Recycle Bin, a Disk for C:, Eject USB and a Disk Meter for C:'; Sensors = @($recycleBin, $disk, $ejectUsb, $diskMeter) }
)

$tempDir = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$vhd = Join-Path $tempDir 'infodock-drive-check.vhd'
$label = 'InfodockCheck'
$attached = $false
$app = $null
$start = Get-LogLines
$filter = $env:RUST_LOG
try {
    if (Test-Path $vhd) { Remove-Item -Force $vhd }
    Invoke-DiskPart @(
        "create vdisk file=`"$vhd`" maximum=64 type=expandable",
        "select vdisk file=`"$vhd`"",
        'attach vdisk',
        'convert mbr',
        'create partition primary',
        "format fs=ntfs quick label=$label",
        'assign'
    )
    $attached = $true
    $letter = $null
    for ($i = 0; $i -lt 50 -and -not $letter; $i++) {
        $found = Get-Volume -FileSystemLabel $label -ErrorAction SilentlyContinue | Where-Object DriveLetter | Select-Object -First 1
        if ($found) { $letter = [string]$found.DriveLetter } else { Start-Sleep -Milliseconds 200 }
    }
    if (-not $letter) { throw "the VHD's volume has no drive letter" }
    Write-Host "The VHD is drive ${letter}:"

    # An item in the drive's Recycle Bin, so reading the bin goes through it.
    $recycled = "${letter}:\recycled.txt"
    Set-Content -Path $recycled -Value 'Infodock drive check'
    [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($recycled, 'OnlyErrorDialogs', 'SendToRecycleBin')
    $binItems = @(Get-ChildItem -Force -Recurse -File -Path "${letter}:\`$Recycle.Bin" -ErrorAction SilentlyContinue).Count
    Add-Result "An item recycled on ${letter}:" ($binItems -gt 0) "$binItems files in its Recycle Bin"

    Add-Result 'Without Infodock, the drive locks within 10 s' (Wait-Lock $letter 10)
    $loop = [DriveLock]::Loop($letter, 10)
    Add-Result 'Without Infodock, every lock of a 10 s loop takes' ($loop.Failures -eq 0) (Format-Loop $loop)
    $held = [System.IO.File]::Open("${letter}:\held.txt", 'OpenOrCreate', 'ReadWrite', 'ReadWrite')
    try { $code = [DriveLock]::LockOnce($letter) } finally { $held.Dispose() }
    Add-Result 'With a file held open on the drive, its lock fails' ($code -ne 0) $(if ($code) { Get-ErrorText $code } else { 'the lock took' })

    foreach ($layout in $layouts) {
        Write-Layout $layout.Sensors
        $before = Get-LogLines
        $env:RUST_LOG = 'info,infodock_lib::platform::windows::drives=debug'
        $app = Start-Process -FilePath $Exe -PassThru
        $env:RUST_LOG = $filter
        Start-Sleep -Seconds 10
        if ($app.HasExited) { throw "Infodock exited during start-up (code $($app.ExitCode))" }
        $loop = [DriveLock]::Loop($letter, $Seconds)
        # The last unlock's watch-again may still be on its way: give it 10 s before the app quits.
        $clock = [System.Diagnostics.Stopwatch]::StartNew()
        do {
            $last = Select-Watch @(Get-LogLines | Select-Object -Skip $before.Count) $letter | Select-Object -Last 1
            if ($last -and $last.Contains("watching ${letter}: again")) { break }
            Start-Sleep -Milliseconds 200
        } while ($clock.Elapsed.TotalSeconds -lt 10)
        & taskkill.exe /pid $app.Id | Out-Null
        if (-not $app.WaitForExit(10000)) { & taskkill.exe /f /t /pid $app.Id | Out-Null }
        $app = $null
        $logged = @(Get-LogLines | Select-Object -Skip $before.Count)
        $drives = @($logged | Select-String -CaseSensitive -Pattern "\b${letter}:")
        Add-Result "With $($layout.Name), every lock takes" ($loop.Failures -eq 0) "$(Format-Loop $loop); $($drives.Count) log lines name ${letter}:"
        $watch = @(Select-Watch $logged $letter)
        $letGo = @($watch | Where-Object { $_.Contains("let go of ${letter}:") }).Count
        $unlocked = @($watch | Where-Object { $_.Contains("watching ${letter}: again: it was unlocked") }).Count
        $last = $watch | Select-Object -Last 1
        $watchedAtEnd = [bool]$last -and $last.Contains("watching ${letter}: again")
        $end = if (-not $last) { 'no such line' } elseif ($watchedAtEnd) { 'watched again' } else { 'let go of' }
        Add-Result "With $($layout.Name), the app lets go of ${letter}: and watches it again after an unlock" ($unlocked -gt 0 -and $watchedAtEnd) "let go of it $letGo times, watched it again $($watch.Count - $letGo) times ($unlocked after an unlock); the last line: $end"
        $first = @($drives | Select-Object -First 10 | ForEach-Object { "   $($_.Line)" })
        $first | ForEach-Object { Write-Host $_; $records.Add($_) }
        # Locks the script makes no more, so the drive must lock at once again.
        Wait-Lock $letter 10 | Out-Null
    }
} catch {
    Write-Warning "The drive release check could not finish: $_"
    $results.Add([pscustomobject]@{ Name = "The check ran ($_)"; Detail = ''; Passed = $false })
} finally {
    $env:RUST_LOG = $filter
    if ($app -and -not $app.HasExited) { & taskkill.exe /f /t /pid $app.Id | Out-Null }
    if ($null -ne $savedConfig) {
        Set-Content -NoNewline -Encoding utf8NoBOM -Path $config -Value $savedConfig
    } elseif (Test-Path $config) {
        Remove-Item -Force $config
    }
    if ($attached) {
        try { Invoke-DiskPart @("select vdisk file=`"$vhd`"", 'detach vdisk') } catch { Write-Warning "$_" }
    }
    Remove-Item -Force $vhd -ErrorAction SilentlyContinue
    $records | Set-Content (Join-Path $OutDir 'steps.txt')
    @(Get-LogLines | Select-Object -Skip $start.Count) | Set-Content (Join-Path $OutDir 'drive-release-check.log')
}

$summary = @('### Drives released when Windows locks them', '', '| Step | Result |', '|---|---|')
$summary += $results | ForEach-Object { "| $($_.Name) | $(if ($_.Passed) { 'passed' } else { '**failed**' })$(if ($_.Detail) { ": $($_.Detail)" }) |" }
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host

$failed = @($results | Where-Object { -not $_.Passed } | ForEach-Object { $_.Name })
if ($failed) {
    $message = 'Drive release checks failed: ' + ($failed -join '; ')
    if ($Enforce) { throw $message } else { Write-Warning $message }
}
