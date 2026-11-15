# Checks a bar kept behind the windows (the default z-order, SPEC §5.3) around Show desktop, with
# the release exe on the runner's desktop: a maximised Notepad covers the bar; Win+D and the
# taskbar's far corner show the desktop, and the bar comes up over it; a right-click or a left
# click on the bar keeps it there, without a flash; a window restored from the taskbar (also one
# that was minimised) covers the bar again. Each step records what is at the bar (WindowFromPoint, the windows above it in the
# z-order), the bar's topmost flag, the foreground window, and a screenshot. The app runs with the
# appbar's debug log, whose `foreground:` and `z-order:` lines are printed. Runs after the memory
# check, whose first run ended the welcome page, and before the installer check, which removes
# the config.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $OutDir,
    # Fail the step when a check fails; without it they are warnings.
    [switch] $Enforce
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int X, Y; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int size);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint cmd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out int value, int size);

    public static int ScreenWidth { get { return GetSystemMetrics(0); } }
    public static int ScreenHeight { get { return GetSystemMetrics(1); } }

    public static string Class(IntPtr hwnd) {
        var name = new StringBuilder(256);
        GetClassNameW(hwnd, name, name.Capacity);
        return name.ToString();
    }
    public static IntPtr Root(IntPtr hwnd) {
        var root = GetAncestor(hwnd, 2 /* GA_ROOT */);
        return root == IntPtr.Zero ? hwnd : root;
    }
    public static IntPtr At(int x, int y) {
        POINT point; point.X = x; point.Y = y;
        return Root(WindowFromPoint(point));
    }
    public static bool Topmost(IntPtr hwnd) { return ((long)GetWindowLongPtr(hwnd, -20) & 0x8) != 0; }
    public static bool Cloaked(IntPtr hwnd) {
        int cloaked;
        return DwmGetWindowAttribute(hwnd, 14 /* DWMWA_CLOAKED */, out cloaked, 4) == 0 && cloaked != 0;
    }
    // A window's class and state, for the record.
    public static string Describe(IntPtr hwnd) {
        if (hwnd == IntPtr.Zero) return "(none)";
        var state = new List<string>();
        if (!IsWindowVisible(hwnd)) state.Add("hidden");
        if (IsIconic(hwnd)) state.Add("minimised");
        if (Cloaked(hwnd)) state.Add("cloaked");
        if (Topmost(hwnd)) state.Add("topmost");
        return Class(hwnd) + (state.Count > 0 ? " (" + string.Join(", ", state) + ")" : "");
    }
    // The shown windows above `hwnd` in the z-order that overlap it, from the nearest up.
    public static List<string> Above(IntPtr hwnd) {
        var found = new List<string>();
        RECT rect;
        GetWindowRect(hwnd, out rect);
        for (var w = GetWindow(hwnd, 3 /* GW_HWNDPREV */); w != IntPtr.Zero; w = GetWindow(w, 3)) {
            if (!IsWindowVisible(w) || IsIconic(w) || Cloaked(w)) continue;
            RECT other;
            GetWindowRect(w, out other);
            if (other.Left < rect.Right && rect.Left < other.Right && other.Top < rect.Bottom && rect.Top < other.Bottom) {
                found.Add(Describe(w));
            }
        }
        return found;
    }

    // Presses the keys in order, then releases them in reverse, as a user does.
    public static void Press(params byte[] keys) {
        foreach (var key in keys) { keybd_event(key, 0, Flags(key), UIntPtr.Zero); Thread.Sleep(40); }
        for (int i = keys.Length - 1; i >= 0; i--) {
            keybd_event(keys[i], 0, Flags(keys[i]) | 2 /* KEYEVENTF_KEYUP */, UIntPtr.Zero);
            Thread.Sleep(40);
        }
    }
    static uint Flags(byte key) { return key == 0x5B /* VK_LWIN */ ? 1u /* KEYEVENTF_EXTENDEDKEY */ : 0u; }

    // A click at (x, y): left, or right with `right`.
    public static void Click(int x, int y, bool right) {
        SetCursorPos(x, y);
        Thread.Sleep(150);
        mouse_event(right ? 8u : 2u /* RIGHTDOWN : LEFTDOWN */, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(right ? 16u : 4u /* RIGHTUP : LEFTUP */, 0, 0, 0, UIntPtr.Zero);
    }
}
'@

$LogDir = Join-Path $env:LOCALAPPDATA 'app.infodock.desktop\logs'
function Get-LogLines {
    if (-not (Test-Path $LogDir)) { return @() }
    @(Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content)
}

function Save-Screen([string] $Name) {
    $bitmap = [System.Drawing.Bitmap]::new([Win]::ScreenWidth, [Win]::ScreenHeight)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen(0, 0, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $OutDir "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

# The taskbar button whose name is like $Like; the names are listed once when none is.
function Get-TaskbarButton([string] $Like) {
    $tray = [Win]::FindWindowW('Shell_TrayWnd', [NullString]::Value)
    if ($tray -eq [IntPtr]::Zero) { return $null }
    $auto = [System.Windows.Automation.AutomationElement]
    $isButton = [System.Windows.Automation.PropertyCondition]::new($auto::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $buttons = @($auto::FromHandle($tray).FindAll([System.Windows.Automation.TreeScope]::Descendants, $isButton))
    $button = $buttons | Where-Object { $_.Current.Name -like $Like } | Select-Object -First 1
    if (-not $button -and -not $script:listed) {
        $script:listed = $true
        Write-Host "Taskbar buttons: $(($buttons | ForEach-Object { "'$($_.Current.Name)'" }) -join ', ')"
    }
    $button
}

# Clicks the window's taskbar button, as a user restores or switches to it.
function Invoke-TaskbarButton([string] $Like) {
    $button = Get-TaskbarButton $Like
    if (-not $button) { throw "no taskbar button like '$Like'" }
    $rect = $button.Current.BoundingRectangle
    [Win]::Click([int]($rect.X + $rect.Width / 2), [int]($rect.Y + $rect.Height / 2), $false)
}

$results = [System.Collections.Generic.List[object]]::new()
$records = [System.Collections.Generic.List[string]]::new()
$script:step = 0

# Records what is at the bar after an action, against what should be: 'bar' (shown) or
# 'covered' (behind the window over it).
function Test-Bar([string] $Name, [string] $Expected, [int] $Wait = 1200, [double] $Along = 0.5) {
    Start-Sleep -Milliseconds $Wait
    $script:step++
    $rect = [Win+RECT]::new()
    [Win]::GetWindowRect($script:bar, [ref] $rect) | Out-Null
    $x = [int]($rect.Left + ($rect.Right - $rect.Left) * $Along)
    $y = [int](($rect.Top + $rect.Bottom) / 2)
    $at = [Win]::At($x, $y)
    $shown = $at -eq $script:bar
    $above = @([Win]::Above($script:bar) | Where-Object { $_ -notlike 'Infodock.*' })
    $passed = if ($Expected -eq 'bar') { $shown } else { -not $shown }
    $line = "$($script:step). $Name -> at the bar: $(if ($shown) { 'the bar' } else { [Win]::Describe($at) }); " +
        "above it: $(if ($above) { $above -join ' > ' } else { 'nothing' }); " +
        "bar topmost: $([Win]::Topmost($script:bar)); foreground: $([Win]::Describe([Win]::GetForegroundWindow()))" +
        $(if ($script:notepad) { "; Notepad: $([Win]::Describe($script:notepad))" } else { '' })
    Write-Host $line
    $records.Add($line)
    Save-Screen ('{0:d2}' -f $script:step)
    $results.Add([pscustomobject]@{ Name = $Name; Expected = $Expected; Passed = $passed })
}

# Whether the bar's z-order stayed as it was since $Before was read: the app logs each change
# (`z-order: …`), so a flash shows even when it is over before a screenshot.
function Test-Kept([object[]] $Before, [string] $Name) {
    $moved = @(Get-LogLines | Select-Object -Skip $Before.Count | Select-String -SimpleMatch 'z-order: ')
    $line = "   $Name -> $(if ($moved) { "moved: $(($moved | ForEach-Object { $_.Line }) -join ' | ')" } else { 'kept' })"
    Write-Host $line
    $records.Add($line)
    $results.Add([pscustomobject]@{ Name = $Name; Expected = 'kept'; Passed = $moved.Count -eq 0 })
}

$start = Get-LogLines
$filter = $env:RUST_LOG
$env:RUST_LOG = 'info,infodock_lib::platform::windows::appbar=debug'
$app = Start-Process -FilePath $Exe -PassThru
$env:RUST_LOG = $filter
$notepadProcess = $null
try {
    Start-Sleep -Seconds 6
    if ($app.HasExited) { throw "Infodock exited during start-up (code $($app.ExitCode))" }
    $script:bar = [Win]::FindWindowW('Infodock.Bar', [NullString]::Value)
    if ($script:bar -eq [IntPtr]::Zero) { throw 'the bar window was not found' }

    # A maximised window over the bar (it reserves no space by default), in the foreground.
    $notepadProcess = Start-Process -FilePath notepad.exe -PassThru
    for ($i = 0; $i -lt 50 -and -not $script:notepad; $i++) {
        Start-Sleep -Milliseconds 200
        $found = [Win]::FindWindowW('Notepad', [NullString]::Value)
        if ($found -ne [IntPtr]::Zero) { $script:notepad = $found }
    }
    if (-not $script:notepad) { throw 'Notepad did not open' }
    [Win]::ShowWindow($script:notepad, 3 <# SW_MAXIMIZE #>) | Out-Null
    Start-Sleep -Milliseconds 500
    [Win]::Click([int]([Win]::ScreenWidth / 2), [int]([Win]::ScreenHeight / 3), $false)
    Test-Bar 'A maximised window covers the bar' 'covered'

    [Win]::Press([byte[]]@(0x5B, 0x44)) # Win+D
    Test-Bar 'Win+D: the bar over the desktop' 'bar'

    $rect = [Win+RECT]::new()
    [Win]::GetWindowRect($script:bar, [ref] $rect) | Out-Null
    $middle = [int](($rect.Top + $rect.Bottom) / 2)
    $before = Get-LogLines
    [Win]::Click([int]($rect.Left + ($rect.Right - $rect.Left) * 0.75), $middle, $true)
    Test-Bar 'Win+D, a right-click on the bar: the bar while its menu is open' 'bar' -Wait 800 -Along 0.25
    [Win]::Press([byte[]]@(0x1B)) # Esc
    Test-Bar 'Win+D, the menu closed with Esc: the bar' 'bar' -Wait 800
    Test-Kept $before 'Win+D, the menu: the bar never left its place'
    # Between the network rates and the logo of the default bar: a click that opens nothing.
    $before = Get-LogLines
    [Win]::Click([int]($rect.Left + ($rect.Right - $rect.Left) * 0.33), $middle, $false)
    Test-Bar 'Win+D, a left click on the bar: the bar' 'bar'
    Test-Kept $before 'Win+D, the left click: the bar never left its place (no flash)'

    Invoke-TaskbarButton '*Notepad*'
    Test-Bar 'After Win+D, the window brought back from the taskbar covers the bar' 'covered'

    [Win]::Press([byte[]]@(0x5B, 0x44))
    Test-Bar 'Win+D again: the bar over the desktop' 'bar'
    [Win]::Press([byte[]]@(0x5B, 0x44))
    Test-Bar 'Win+D once more: the windows back over the bar' 'covered'

    [Win]::ShowWindow($script:notepad, 6 <# SW_MINIMIZE #>) | Out-Null
    Test-Bar 'Every window minimised: the bar' 'bar'
    [Win]::Press([byte[]]@(0x5B, 0x44))
    Test-Bar 'Every window minimised, Win+D: the bar' 'bar'
    Invoke-TaskbarButton '*Notepad*'
    Test-Bar 'Then the minimised window restored from the taskbar covers the bar' 'covered'

    $tray = [Win]::FindWindowW('Shell_TrayWnd', [NullString]::Value)
    $trayRect = [Win+RECT]::new()
    [Win]::GetWindowRect($tray, [ref] $trayRect) | Out-Null
    [Win]::Click($trayRect.Right - 2, [int](($trayRect.Top + $trayRect.Bottom) / 2), $false)
    Test-Bar "The taskbar's far corner (Show desktop): the bar over the desktop" 'bar'
    Invoke-TaskbarButton '*Notepad*'
    Test-Bar 'Then the window brought back from the taskbar covers the bar' 'covered'
} catch {
    Write-Warning "The Show desktop check could not finish: $_"
    $results.Add([pscustomobject]@{ Name = "The check ran ($_)"; Expected = ''; Passed = $false })
} finally {
    if ($notepadProcess -and -not $notepadProcess.HasExited) { $notepadProcess.Kill() }
    if (-not $app.HasExited) {
        & taskkill.exe /pid $app.Id | Out-Null
        if (-not $app.WaitForExit(10000)) { & taskkill.exe /f /t /pid $app.Id | Out-Null }
    }
    $lines = @(Get-LogLines | Select-Object -Skip $start.Count)
    $records | Set-Content (Join-Path $OutDir 'steps.txt')
    $lines | Set-Content (Join-Path $OutDir 'show-desktop-check.log')
    Write-Host '== What the app logged'
    $lines | Write-Host
}

$summary = @('### Show desktop and a bar kept behind the windows (SPEC §5.3)', '', '| Step | Result |', '|---|---|')
$summary += $results | ForEach-Object { "| $($_.Name) | $(if ($_.Passed) { 'passed' } else { '**failed**' }) |" }
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host

$failed = @($results | Where-Object { -not $_.Passed } | ForEach-Object { $_.Name })
if ($failed) {
    $message = 'Show desktop checks failed: ' + ($failed -join '; ')
    if ($Enforce) { throw $message } else { Write-Warning $message }
}
