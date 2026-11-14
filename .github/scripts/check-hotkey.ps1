# Checks a hotkey another app holds while Settings records one (SPEC §6.5), with the release
# exe on the runner's desktop. A thread of this script holds Ctrl+Alt+J with RegisterHotKey, as
# another app would. Settings opens (a second launch, on General), the hotkey button is clicked
# with the mouse and Ctrl+Alt+J is injected: Settings should find it taken and the holder should
# never get it. Then Infodock's own hotkey (the default, Ctrl+Alt+Shift+I), pressed while
# recording, should be found taken too. The app runs with the keyboard hook's trace (RUST_LOG);
# the lines it logged are printed. Runs after the memory check, whose first run ended the
# welcome page, and before the installer check, which removes the config. It found S14x2:
# Windows called no low-level keyboard hook while Infodock's was the newest, as long as the app
# read the keyboard's raw input (D120); a hook of another app installed after Infodock's hid that.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $OutDir,
    # Fail the step when a check fails; without it they are warnings.
    [switch] $Enforce
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Threading;

public static class Desktop {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")]
    static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")]
    static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);

    // Presses the keys in order, then releases them in reverse, as a user does.
    public static void Press(params byte[] keys) {
        foreach (var key in keys) { keybd_event(key, 0, 0, UIntPtr.Zero); Thread.Sleep(40); }
        for (int i = keys.Length - 1; i >= 0; i--) {
            keybd_event(keys[i], 0, 2 /* KEYEVENTF_KEYUP */, UIntPtr.Zero);
            Thread.Sleep(40);
        }
    }

    // A left click at (x, y), in this process's screen coordinates.
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(150);
        mouse_event(2 /* LEFTDOWN */, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(4 /* LEFTUP */, 0, 0, 0, UIntPtr.Zero);
    }
}

// Holds a hotkey on a thread of its own, as another app does, and counts the WM_HOTKEYs it gets.
public sealed class Holder {
    [StructLayout(LayoutKind.Sequential)]
    struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam; public uint time; public int x; public int y; }
    [DllImport("user32.dll")] static extern bool RegisterHotKey(IntPtr hwnd, int id, uint mods, uint vk);
    [DllImport("user32.dll")] static extern bool UnregisterHotKey(IntPtr hwnd, int id);
    [DllImport("user32.dll")] static extern bool PeekMessageW(out MSG msg, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] static extern int GetMessageW(out MSG msg, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] static extern bool PostThreadMessageW(uint thread, uint msg, IntPtr w, IntPtr l);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();

    readonly Thread thread;
    readonly ManualResetEventSlim ready = new ManualResetEventSlim();
    uint id;
    volatile int hotkeys;
    public bool Held;
    public int Hotkeys { get { return hotkeys; } }

    public Holder(uint modifiers, uint vk) {
        thread = new Thread(() => {
            MSG msg;
            PeekMessageW(out msg, IntPtr.Zero, 0, 0, 0); // the thread's queue, for WM_QUIT
            id = GetCurrentThreadId();
            Held = RegisterHotKey(IntPtr.Zero, 1, modifiers, vk);
            ready.Set();
            while (GetMessageW(out msg, IntPtr.Zero, 0, 0) > 0) {
                if (msg.message == 0x0312 /* WM_HOTKEY */) hotkeys++;
            }
            UnregisterHotKey(IntPtr.Zero, 1);
        });
        thread.IsBackground = true;
        thread.Start();
        ready.Wait();
    }

    public void Stop() {
        PostThreadMessageW(id, 0x0012 /* WM_QUIT */, IntPtr.Zero, IntPtr.Zero);
        thread.Join(5000);
    }
}
'@

$LogDir = Join-Path $env:LOCALAPPDATA 'app.infodock.desktop\logs'
function Get-LogLines {
    if (-not (Test-Path $LogDir)) { return @() }
    @(Get-ChildItem $LogDir -Filter 'infodock*.log' | Sort-Object Name | Get-Content)
}
# The lines logged since $Before was read.
function Get-NewLines([object[]] $Before) {
    @(Get-LogLines | Select-Object -Skip $Before.Count)
}
function Wait-ForLine([object[]] $Before, [string] $Pattern, [int] $Ms = 3000) {
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    while ($clock.ElapsedMilliseconds -lt $Ms) {
        if (Get-NewLines $Before | Select-String -Pattern $Pattern -Quiet) { return $true }
        Start-Sleep -Milliseconds 200
    }
    $false
}

# The hotkey button on Settings' General page, found by its accessible label.
function Get-HotkeyButton([IntPtr] $Window) {
    $auto = [System.Windows.Automation.AutomationElement]
    $named = [System.Windows.Automation.PropertyCondition]::new($auto::NameProperty, 'Global hotkey')
    $button = [System.Windows.Automation.PropertyCondition]::new($auto::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $both = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@($named, $button))
    $auto::FromHandle($Window).FindFirst([System.Windows.Automation.TreeScope]::Descendants, $both)
}

# Clicks the hotkey button, which starts recording, and presses $Keys; $true when Settings found
# the combination $Name taken.
function Test-Taken([IntPtr] $Window, [string] $Name, [byte[]] $Keys) {
    $button = Get-HotkeyButton $Window
    if (-not $button) { Write-Warning 'The hotkey button was not found'; return $false }
    $rect = $button.Current.BoundingRectangle
    $before = Get-LogLines
    [Desktop]::Click([int]($rect.X + $rect.Width / 2), [int]($rect.Y + $rect.Height / 2))
    if (-not (Wait-ForLine $before 'watching for hotkeys other apps hold')) {
        Write-Warning "The click on the hotkey button at $rect did not start the watch"
    }
    [Desktop]::Press($Keys)
    $taken = Wait-ForLine $before ([regex]::Escape("$Name is taken"))
    # Still recording if it was not found: Esc ends it.
    if (-not $taken) { [Desktop]::Press([byte[]]@(0x1B)) }
    Start-Sleep -Milliseconds 300
    $taken
}

$results = [ordered]@{}
$start = Get-LogLines
$holder = [Holder]::new(0x3 <# MOD_ALT | MOD_CONTROL #>, 0x4A <# J #>)
if (-not $holder.Held) { Write-Warning 'Ctrl+Alt+J was already held on the runner; the check uses that holder' }
$filter = $env:RUST_LOG
$env:RUST_LOG = 'info,infodock_lib::platform::windows::hotkeys=trace,infodock_lib::settings=debug'
$app = Start-Process -FilePath $Exe -PassThru
$env:RUST_LOG = $filter
try {
    Start-Sleep -Seconds 8
    if ($app.HasExited) { throw "Infodock exited during start-up (code $($app.ExitCode))" }
    # A second launch opens Settings on General (SPEC §4.1).
    $second = Start-Process -FilePath $Exe -PassThru
    if (-not $second.WaitForExit(15000)) { Write-Warning 'The second instance did not exit' }
    $window = [IntPtr]::Zero
    for ($i = 0; $i -lt 50 -and $window -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 200
        # [NullString]: PowerShell would pass $null to a string parameter as "".
        $window = [Desktop]::FindWindowW([NullString]::Value, 'Infodock Settings')
    }
    if ($window -eq [IntPtr]::Zero) { throw 'Settings did not open' }
    Start-Sleep -Seconds 2

    $hotkeysBefore = $holder.Hotkeys
    $results['Ctrl+Alt+J, held by another app, found taken'] = Test-Taken $window 'Ctrl+Alt+J' @(0x11, 0x12, 0x4A)
    $results['Ctrl+Alt+J kept from the app that holds it'] = $holder.Hotkeys -eq $hotkeysBefore
    $results["Infodock's own hotkey found taken"] = Test-Taken $window 'Ctrl+Alt+Shift+I' @(0x11, 0x12, 0x10, 0x49)

    [Desktop]::PostMessageW($window, 0x0010 <# WM_CLOSE #>, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Seconds 1
} catch {
    Write-Warning "The hotkey check could not run: $_"
    $results['The check ran'] = $false
} finally {
    if (-not $app.HasExited) {
        & taskkill.exe /pid $app.Id | Out-Null
        if (-not $app.WaitForExit(10000)) { & taskkill.exe /f /t /pid $app.Id | Out-Null }
    }
    $holder.Stop()
    $lines = Get-NewLines $start
    $lines | Set-Content (Join-Path $OutDir 'hotkey-check.log')
    Write-Host '== What the app logged'
    $lines | Write-Host
}

$summary = @('### Hotkey held by another app (SPEC §6.5)', '', '| Check | Result |', '|---|---|')
$summary += $results.GetEnumerator() | ForEach-Object { "| $($_.Key) | $(if ($_.Value) { 'passed' } else { '**failed**' }) |" }
if ($env:GITHUB_STEP_SUMMARY) { $summary -join "`n" | Out-File -Append -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary -join "`n" | Write-Host

$failed = @($results.GetEnumerator() | Where-Object { -not $_.Value } | ForEach-Object { $_.Key })
if ($failed) {
    $message = 'Hotkey checks failed: ' + ($failed -join '; ')
    if ($Enforce) { throw $message } else { Write-Warning $message }
}
