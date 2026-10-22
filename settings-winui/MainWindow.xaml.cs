using System.Runtime.InteropServices;
using InfodockSettings.Pages;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;

namespace InfodockSettings;

public sealed partial class MainWindow : Window
{
    /// <summary>The open window, for the Appearance page's colour scheme.</summary>
    public static MainWindow? Instance { get; private set; }

    public MainWindow()
    {
        Instance = this;
        InitializeComponent();
        Title = I18n.T("settings.title");
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "icon.ico"));
        TitleIcon.Source = new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "icon-32.png")));
        Place();
        ApplyTheme(Startup.Theme switch
        {
            "light" => ElementTheme.Light,
            "dark" => ElementTheme.Dark,
            _ => SchemeTheme(Config.Current.Choice("colorScheme", "system", "system", "light", "dark")),
        });
        Nav.SelectedItem = Nav.MenuItems.OfType<NavigationViewItem>().FirstOrDefault(item => item.Tag as string == Startup.Page)
            ?? Nav.MenuItems[0];
        Startup.Step("window");
    }

    /// <summary>The theme of a colour scheme's index (system, light, dark).</summary>
    public static ElementTheme SchemeTheme(int scheme) => scheme switch
    {
        1 => ElementTheme.Light,
        2 => ElementTheme.Dark,
        _ => ElementTheme.Default,
    };

    /// <summary>Light, dark or as Windows is; the caption buttons follow.</summary>
    public void ApplyTheme(ElementTheme theme)
    {
        Root.RequestedTheme = theme;
        var dark = Root.ActualTheme == ElementTheme.Dark;
        // The caption buttons are the system's on the extended title bar; WinUI's caption colours.
        var titleBar = AppWindow.TitleBar;
        var foreground = dark ? Colors.White : Colors.Black;
        titleBar.ButtonBackgroundColor = Colors.Transparent;
        titleBar.ButtonInactiveBackgroundColor = Colors.Transparent;
        titleBar.ButtonForegroundColor = foreground;
        titleBar.ButtonHoverForegroundColor = foreground;
        titleBar.ButtonPressedForegroundColor = foreground;
        titleBar.ButtonInactiveForegroundColor = dark ? ColorHelper.FromArgb(0x5D, 0xFF, 0xFF, 0xFF) : ColorHelper.FromArgb(0x5C, 0, 0, 0);
        titleBar.ButtonHoverBackgroundColor = dark ? ColorHelper.FromArgb(0x0F, 0xFF, 0xFF, 0xFF) : ColorHelper.FromArgb(0x09, 0, 0, 0);
        titleBar.ButtonPressedBackgroundColor = dark ? ColorHelper.FromArgb(0x0A, 0xFF, 0xFF, 0xFF) : ColorHelper.FromArgb(0x06, 0, 0, 0);
    }

    /// <summary>
    /// As large as the Slint window, which opens at 760 × 560 logical px below a 32 px system
    /// title bar; here the title bar is part of the client area. Centred on the work area.
    /// </summary>
    void Place()
    {
        var scale = GetDpiForWindow(Win32Interop.GetWindowFromWindowId(AppWindow.Id)) / 96.0;
        AppWindow.ResizeClient(new SizeInt32((int)(760 * scale), (int)((560 + 32) * scale)));
        var size = AppWindow.Size;
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        AppWindow.Move(new PointInt32(area.X + (area.Width - size.Width) / 2, area.Y + (area.Height - size.Height) / 2));
    }

    void OnSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if ((args.SelectedItem as NavigationViewItem)?.Tag is not string page)
            return;
        PageTitle.Text = I18n.T($"settings.nav.{page}");
        var type = page switch
        {
            "general" => typeof(GeneralPage),
            "appearance" => typeof(AppearancePage),
            _ => typeof(PlaceholderPage),
        };
        PageFrame.Navigate(type, null, new EntranceNavigationTransitionInfo());
    }

    void OnRootLoaded(object sender, RoutedEventArgs e)
    {
        Startup.Step("loaded");
        CompositionTarget.Rendering += OnFirstRendering;
    }

    void OnFirstRendering(object? sender, object e)
    {
        CompositionTarget.Rendering -= OnFirstRendering;
        Startup.FirstFrame();
    }

    [DllImport("user32.dll")]
    static extern uint GetDpiForWindow(nint hwnd);
}
