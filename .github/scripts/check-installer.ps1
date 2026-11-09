# Checks the NSIS installer (SPEC §15.3, the M4 exit criterion): it asks for no elevation,
# installs silently per user (no Program Files, no HKLM), with the Start Menu shortcut carrying
# the AppUserModelID, and starts the app after installing (/R). The app's first run takes an
# existing sign-in entry for a missing exe into its setting and points it at the installed exe
# (§10.1, §11.5). The uninstaller removes the app, its shortcuts, its registration (the install
# location under HKCU too), the sign-in entry and Task Manager's mark for it. A second
# silent install with /AUTOSTART writes the sign-in entry for the installed exe and clears Task
# Manager's mark for it, without starting the app; its uninstall removes the entry again.
# Before all that, the interactive installer's first two pages are saved as screenshots, for its
# images (Appendix C), and it is closed there, before anything is installed.
# Runs after the memory check: it removes that run's config, so the installed app runs first.
param(
    [Parameter(Mandatory)] [string] $Setup,
    [Parameter(Mandatory)] [string] $OutDir
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
# The interactive installer's window: where it is, and its Next button pressed. UI Automation
# finds the button but offers no Invoke on it, so the dialog gets its IDOK command.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class InstallerWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
    // NSIS's Next button is IDOK; WM_COMMAND with BN_CLICKED (0) from it.
    public static bool Next(IntPtr dialog) {
        return PostMessage(dialog, 0x0111, (IntPtr)1, GetDlgItem(dialog, 1));
    }
    // Left, top, right, bottom; none when the window is gone.
    public static int[] Edges(IntPtr window) {
        Rect r;
        if (!GetWindowRect(window, out r)) { return null; }
        return new int[] { r.Left, r.Top, r.Right, r.Bottom };
    }
}
'@

# The window's place on the screen; empty (the whole screen) when it cannot be read.
function Get-WindowBounds([IntPtr] $Window) {
    $edges = [InstallerWindow]::Edges($Window)
    if (-not $edges) { return [System.Drawing.Rectangle]::Empty }
    [System.Drawing.Rectangle]::FromLTRB($edges[0], $edges[1], $edges[2], $edges[3])
}

$identifier = 'app.infodock.desktop'
$product = 'Infodock'
# `autostart::RUN_VALUE`.
$runValue = 'Infodock'
$installDir = Join-Path $env:LOCALAPPDATA $product
$exe = Join-Path $installDir 'infodock.exe'
$uninstaller = Join-Path $installDir 'uninstall.exe'
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$approvedKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$appIdKey = "HKCU:\Software\Classes\AppUserModelId\$identifier"
# Where the template keeps the install location.
$publisherKey = "HKCU:\Software\jurakovic\$product"
$programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Join-Path $programs "$product\$product.lnk"
$desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) "$product.lnk"
$configDir = Join-Path $env:APPDATA $identifier

$failures = [System.Collections.Generic.List[string]]::new()
function Check([bool] $Ok, [string] $What) {
    if ($Ok) {
        Write-Host "ok: $What"
    } else {
        Write-Host "FAILED: $What"
        $failures.Add($What)
    }
}

# The primary screen, or the part of it in `$Area`. (PowerShell's names ignore case, so the
# screen's rectangle has a name of its own.)
function Save-Screenshot([string] $Path, [System.Drawing.Rectangle] $Area = [System.Drawing.Rectangle]::Empty) {
    $shot = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    if (-not $Area.IsEmpty) { $shot = [System.Drawing.Rectangle]::Intersect($shot, $Area) }
    $bitmap = New-Object System.Drawing.Bitmap $shot.Width, $shot.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($shot.Location, [System.Drawing.Point]::Empty, $shot.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
}

# The welcome page (the sidebar image) and the next one (the header image), then closed. Only
# for looking at: what goes wrong here is a warning.
function Save-InstallerPages {
    $installer = Start-Process -FilePath $Setup -PassThru
    try {
        $deadline = (Get-Date).AddSeconds(30)
        while ($installer.MainWindowHandle -eq 0 -and -not $installer.HasExited -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 250
            $installer.Refresh()
        }
        if ($installer.MainWindowHandle -eq 0) {
            Write-Warning 'The interactive installer showed no window within 30 s'
            return
        }
        $window = $installer.MainWindowHandle
        Start-Sleep -Seconds 2
        Save-Screenshot (Join-Path $OutDir 'installer-welcome.png') (Get-WindowBounds $window)
        if (-not [InstallerWindow]::Next($window)) {
            Write-Warning "The installer's Next button could not be pressed"
            return
        }
        Start-Sleep -Seconds 2
        Save-Screenshot (Join-Path $OutDir 'installer-page.png') (Get-WindowBounds $window)
    } catch {
        Write-Warning "The installer's pages could not be saved: $_"
    } finally {
        if (-not $installer.HasExited) {
            Stop-Process -Id $installer.Id -Force -ErrorAction SilentlyContinue
            $null = $installer.WaitForExit(10000)
        }
    }
}

function Get-RunEntry {
    (Get-ItemProperty -Path $runKey -Name $runValue -ErrorAction SilentlyContinue).$runValue
}

function Get-ApprovedMark {
    (Get-ItemProperty -Path $approvedKey -Name $runValue -ErrorAction SilentlyContinue).$runValue
}

# Runs the installer silently with these arguments and waits for it.
function Install-App([string[]] $Arguments) {
    # WaitForExit, not -Wait, which would wait for the app the installer starts too.
    $install = Start-Process -FilePath $Setup -ArgumentList $Arguments -PassThru
    # Holding the handle keeps the exit code readable after the process ends.
    $null = $install.Handle
    if (-not $install.WaitForExit(120000)) { throw 'The installer did not finish within 2 minutes' }
    Check ($install.ExitCode -eq 0) "the silent install ($($Arguments -join ' ')) ends with 0 (got $($install.ExitCode))"
}

# Runs the uninstaller silently and waits until the app and its registration are gone.
function Uninstall-App {
    $uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru
    if (-not $uninstall.WaitForExit(60000)) { throw 'The uninstaller did not finish within a minute' }
    # It runs a copy of itself from the temp folder: wait for that to finish.
    $deadline = (Get-Date).AddSeconds(60)
    while (((Test-Path $exe) -or (Test-Path $uninstallKey)) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
    }
    Start-Sleep -Seconds 2
}

function Get-InstalledApp {
    Get-Process -Name 'infodock' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
}

# No elevation: `RequestExecutionLevel user` gives the installer's manifest `asInvoker`.
$bytes = [System.IO.File]::ReadAllBytes($Setup)
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
Check ($text -match 'requestedExecutionLevel\s+level="asInvoker"') 'the installer asks for no elevation'

Write-Host '== Saving the installer pages'
Save-InstallerPages
Check (-not (Test-Path $exe)) 'closing the interactive installer installed nothing'

# A sign-in entry left from elsewhere, and no config: the installed app's first run.
if (Test-Path $configDir) { Remove-Item -Recurse -Force $configDir }
New-ItemProperty -Path $runKey -Name $runValue -Value '"C:\Elsewhere\infodock.exe"' -PropertyType String -Force | Out-Null

Write-Host "== Installing $Setup"
Install-App '/S', '/R'
Check (Test-Path $exe) "the app is installed in $installDir"
Check (Test-Path $uninstaller) 'the uninstaller is installed'
foreach ($dir in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:ProgramW6432) | Where-Object { $_ } | Select-Object -Unique) {
    Check (-not (Test-Path (Join-Path $dir $product))) "nothing in $dir"
}
Check (Test-Path $uninstallKey) 'listed in Apps & features for this user (HKCU)'
$machineKeys = @(
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product",
    "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$product",
    'HKLM:\Software\jurakovic',
    'HKLM:\Software\WOW6432Node\jurakovic'
)
foreach ($key in $machineKeys) {
    Check (-not (Test-Path $key)) "nothing at $key"
}
Check (Test-Path $shortcut) "the Start Menu shortcut $shortcut"
$allUsers = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\$product"
Check (-not (Test-Path $allUsers)) 'no Start Menu shortcut for all users'
if (Test-Path $shortcut) {
    $folder = (New-Object -ComObject Shell.Application).Namespace((Split-Path $shortcut))
    $aumid = $folder.ParseName((Split-Path $shortcut -Leaf)).ExtendedProperty('System.AppUserModel.ID')
    Check ($aumid -eq $identifier) "the shortcut's AppUserModelID is $identifier (got '$aumid')"
}

# /R started the app after installing.
$deadline = (Get-Date).AddSeconds(15)
while (-not (Get-InstalledApp) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
$app = Get-InstalledApp
Check ([bool] $app) 'the installer started the installed app'
Start-Sleep -Seconds 10
Save-Screenshot (Join-Path $OutDir 'installed.png')
Check ([bool] (Get-InstalledApp)) 'the installed app keeps running'
$entry = Get-RunEntry
Check ($entry -eq "`"$exe`"") "the first run took the sign-in entry and points it at the installed app (got '$entry')"

# Graceful quit, so the config is written.
foreach ($process in @(Get-InstalledApp)) {
    & taskkill.exe /pid $process.Id | Out-Null
    if (-not $process.WaitForExit(10000)) {
        Write-Warning 'Infodock did not quit within 10 s; killing it'
        & taskkill.exe /f /t /pid $process.Id | Out-Null
    }
}
$config = Join-Path $configDir 'config.json'
$autostart = (Test-Path $config) -and ((Get-Content $config -Raw | ConvertFrom-Json).settings.autostart -eq $true)
Check $autostart 'the config has autostart on'

# Task Manager's Disabled mark, which the uninstaller removes with the entry. -Force creates the
# parent keys too; only for a key that is not there, since it would clear one.
if (-not (Test-Path $approvedKey)) { New-Item -Path $approvedKey -Force | Out-Null }
$disabled = [byte[]](3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
New-ItemProperty -Path $approvedKey -Name $runValue -Value $disabled -PropertyType Binary -Force | Out-Null
Check (Test-Path $publisherKey) "the install location is kept at $publisherKey"

Write-Host '== Uninstalling'
Uninstall-App
Check (-not (Test-Path $installDir)) "$installDir is removed"
Check (-not (Test-Path $uninstallKey)) 'no longer in Apps & features'
Check (-not (Test-Path $shortcut)) 'the Start Menu shortcut is removed'
Check (-not (Test-Path $desktopShortcut)) 'the desktop shortcut is removed'
Check ($null -eq (Get-RunEntry)) 'the sign-in entry is removed'
Check ($null -eq (Get-ApprovedMark)) "Task Manager's mark is removed"
Check (-not (Test-Path $appIdKey)) 'the AppUserModelID registration is removed'
Check (-not (Test-Path $publisherKey)) "$publisherKey is removed"

# A deployment's install: /AUTOSTART writes the entry and clears Task Manager's Disabled mark.
Write-Host '== Installing with /AUTOSTART'
New-ItemProperty -Path $approvedKey -Name $runValue -Value $disabled -PropertyType Binary -Force | Out-Null
Install-App '/S', '/AUTOSTART'
$entry = Get-RunEntry
Check ($entry -eq "`"$exe`"") "/AUTOSTART wrote the sign-in entry for the installed app (got '$entry')"
Check ($null -eq (Get-ApprovedMark)) "/AUTOSTART cleared Task Manager's mark"
Check (-not (Get-InstalledApp)) 'without /R the installer did not start the app'
Write-Host '== Uninstalling'
Uninstall-App
Check (-not (Test-Path $installDir)) "$installDir is removed again"
Check ($null -eq (Get-RunEntry)) 'the sign-in entry written by /AUTOSTART is removed'

if ($failures.Count -gt 0) {
    throw "Installer check failed: $($failures -join '; ')"
}
Write-Host '== Installer check passed'
