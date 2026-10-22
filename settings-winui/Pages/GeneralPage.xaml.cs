using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Windows.System;
using Windows.UI.Core;

namespace InfodockSettings.Pages;

/// <summary>SPEC §10.1, the rows of the Slint window's General page.</summary>
public sealed partial class GeneralPage : Page
{
    string hotkey;
    bool recording;

    public GeneralPage()
    {
        InitializeComponent();
        var config = Config.Current;
        Autostart.IsOn = config.Flag("autostart", false);
        hotkey = config.Text("hotkey") ?? "Ctrl+Alt+Shift+I";
        HotkeyAction.SelectedIndex = config.Choice("hotkeyAction", "bringToFront", "bringToFront", "toggleVisibility");
        ShowTrayIcon.IsOn = config.Flag("showTrayIcon", true);
        Locked.IsOn = config.Flag("locked", false);
        PopupsEnabled.IsOn = config.Flag("popupsEnabled", true);
        ClicksWhenLocked.SelectedIndex = config.Choice("clicksWhenLocked", "always", "always", "altOnly");
        Spacing.Value = config.Number("sensorSpacing") ?? 10;
        ShowSpacing();
        NewSensorMenu.SelectedIndex = config.Choice("newSensorMenu", "dialog", "dialog", "submenu");
        ShowHotkey();
    }

    void OnSpacingChanged(object sender, RangeBaseValueChangedEventArgs e) => ShowSpacing();

    void ShowSpacing()
    {
        // The slider reports its minimum while the page is still being built.
        if (SpacingValue is not null && Spacing is not null)
            SpacingValue.Text = I18n.T("common.px", "value", (int)Spacing.Value);
    }

    void ShowHotkey()
    {
        Hotkey.Content = recording ? I18n.T("settings.general.hotkey.recording") : hotkey;
        Hotkey.Style = recording ? (Style)Application.Current.Resources["AccentButtonStyle"] : null;
    }

    void OnHotkeyClick(object sender, RoutedEventArgs e)
    {
        recording = !recording;
        ShowHotkey();
    }

    void OnHotkeyLostFocus(object sender, RoutedEventArgs e)
    {
        recording = false;
        ShowHotkey();
    }

    /// <summary>While recording, the next key with its modifiers is the hotkey; Esc cancels.</summary>
    void OnHotkeyKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (!recording)
            return;
        e.Handled = true;
        if (e.Key == VirtualKey.Escape)
        {
            recording = false;
            ShowHotkey();
            return;
        }
        if (e.Key is VirtualKey.Control or VirtualKey.Menu or VirtualKey.Shift or VirtualKey.LeftWindows
            or VirtualKey.RightWindows or VirtualKey.LeftControl or VirtualKey.RightControl
            or VirtualKey.LeftMenu or VirtualKey.RightMenu or VirtualKey.LeftShift or VirtualKey.RightShift)
            return;
        var parts = new List<string>();
        if (IsDown(VirtualKey.Control))
            parts.Add("Ctrl");
        if (IsDown(VirtualKey.Menu))
            parts.Add("Alt");
        if (IsDown(VirtualKey.Shift))
            parts.Add("Shift");
        if (IsDown(VirtualKey.LeftWindows) || IsDown(VirtualKey.RightWindows))
            parts.Add("Win");
        parts.Add(KeyName(e.Key));
        hotkey = string.Join("+", parts);
        recording = false;
        ShowHotkey();
    }

    static bool IsDown(VirtualKey key) =>
        (InputKeyboardSource.GetKeyStateForCurrentThread(key) & CoreVirtualKeyStates.Down) != 0;

    static string KeyName(VirtualKey key)
    {
        var code = (int)key;
        if (code >= (int)VirtualKey.Number0 && code <= (int)VirtualKey.Number9)
            return (code - (int)VirtualKey.Number0).ToString();
        return key.ToString();
    }
}
