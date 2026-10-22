using Microsoft.UI.Xaml;

namespace InfodockSettings;

public partial class App : Application
{
    Window? window;

    public App()
    {
        Startup.Step("app");
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        window = new MainWindow();
        window.Activate();
        Startup.Step("activated");
    }
}
