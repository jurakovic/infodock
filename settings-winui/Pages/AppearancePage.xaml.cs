using System.Globalization;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace InfodockSettings.Pages;

/// <summary>
/// SPEC §10.3, the rows of the Slint window's Appearance page. The classic theme's skin gallery
/// and packs are left out: their previews are drawn by the bar.
/// </summary>
public sealed partial class AppearancePage : Page
{
    static readonly string[] Materials = ["none", "acrylic", "mica"];

    // As stored: null is the theme's default.
    string? color;
    int? opacity;
    bool loading = true;

    public AppearancePage()
    {
        InitializeComponent();
        var config = Config.Current;
        Theme.SelectedIndex = config.Choice("theme", "modern", "modern", "classic");
        ColorScheme.SelectedIndex = config.Choice("colorScheme", "system", "system", "light", "dark");
        ColorMode.SelectedIndex = config.Choice("background.colorMode", "custom", "standard", "custom");
        color = config.Text("background.color");
        opacity = config.Number("background.opacity");
        ShowThemeDefaults();
        var material = config.Text("background.material");
        Material.SelectedIndex = material is null ? 0 : Array.IndexOf(Materials, material) + 1;
        var available = new UISettings().AdvancedEffectsEnabled;
        Material.IsEnabled = available;
        MaterialCard.Description = I18n.T(available ? "settings.appearance.material.help" : "settings.appearance.material.off");
        loading = false;
    }

    bool Classic => Theme.SelectedIndex == 1;

    bool Light => ColorScheme.SelectedIndex switch
    {
        1 => true,
        2 => false,
        _ => Application.Current.RequestedTheme == ApplicationTheme.Light,
    };

    // The theme's defaults (ThemeDefaults in config/model.rs); the modern bar's light look has
    // its own background.
    string DefaultColor => Classic ? "#2A2A2E" : Light ? "#F3F3F3" : "#1C1C1F";

    int DefaultOpacity => Classic ? 90 : 80;

    string DefaultMaterial => Classic ? "none" : "acrylic";

    /// <summary>The rows whose text or value comes from the theme.</summary>
    void ShowThemeDefaults()
    {
        ColorModeCard.Description = Classic ? I18n.T("settings.appearance.colorMode.classicHelp") : null!;
        ColorCard.Visibility = ColorMode.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
        ColorCard.Description = I18n.T("settings.popups.default", "value", DefaultColor);
        ShowColor();
        OpacityCard.Description = Classic
            ? I18n.T("settings.appearance.opacity.classicHelp", "value", DefaultOpacity)
            : I18n.T("settings.popups.opacity.default", "value", DefaultOpacity);
        OpacitySlider.Value = opacity ?? DefaultOpacity;
        ShowOpacity();
        var selected = Math.Max(0, Material.SelectedIndex);
        Material.Items.Clear();
        Material.Items.Add(I18n.T("common.themeDefault", "value", I18n.T($"settings.appearance.material.{DefaultMaterial}")));
        foreach (var id in Materials)
            Material.Items.Add(I18n.T($"settings.appearance.material.{id}"));
        Material.SelectedIndex = selected;
    }

    void OnThemeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!loading)
            ShowThemeDefaults();
    }

    void OnColorSchemeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (loading)
            return;
        MainWindow.Instance?.ApplyTheme(MainWindow.SchemeTheme(ColorScheme.SelectedIndex));
        ShowThemeDefaults();
    }

    void OnColorModeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!loading)
            ShowThemeDefaults();
    }

    void ShowColor()
    {
        var text = color ?? DefaultColor;
        if (ColorText.Text != text)
            ColorText.Text = text;
        if (Parse(text) is { } shown)
        {
            SwatchFill.Background = new SolidColorBrush(shown);
            Picker.Color = shown;
        }
        ColorReset.IsEnabled = color is not null;
    }

    /// <summary>Accepted as soon as the typed text is a colour, as in the Slint window.</summary>
    void OnColorTextChanged(object sender, TextChangedEventArgs e)
    {
        // TextChanged comes after the change, also after ShowColor's.
        if (loading || ColorText.Text == (color ?? DefaultColor) || Parse(ColorText.Text) is null)
            return;
        color = ColorText.Text;
        ShowColor();
    }

    void OnPickerColorChanged(ColorPicker sender, ColorChangedEventArgs args)
    {
        var text = Format(args.NewColor);
        if (loading || text == (color ?? DefaultColor))
            return;
        color = text;
        ShowColor();
    }

    void OnColorReset(object sender, RoutedEventArgs e)
    {
        color = null;
        ShowColor();
    }

    void OnOpacityChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (!loading && (int)e.NewValue != (opacity ?? DefaultOpacity))
            opacity = (int)e.NewValue;
        ShowOpacity();
    }

    void ShowOpacity()
    {
        // The slider reports its first value while the page is still being built.
        if (OpacityValue is null || OpacitySlider is null || OpacityReset is null)
            return;
        OpacityValue.Text = I18n.T("common.percent", "value", (int)OpacitySlider.Value);
        OpacityReset.IsEnabled = opacity is not null;
    }

    void OnOpacityReset(object sender, RoutedEventArgs e)
    {
        opacity = null;
        OpacitySlider.Value = DefaultOpacity;
        ShowOpacity();
    }

    static Color? Parse(string text) =>
        text.Length == 7 && text[0] == '#'
        && uint.TryParse(text.AsSpan(1), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var rgb)
            ? ColorHelper.FromArgb(0xFF, (byte)(rgb >> 16), (byte)(rgb >> 8), (byte)rgb)
            : null;

    static string Format(Color color) => $"#{color.R:X2}{color.G:X2}{color.B:X2}";
}
