using System.Diagnostics;

namespace InfodockSettings;

/// <summary>
/// The command line and the start-up log. The log line has the bar's form (SPEC §14.1): the
/// time from the process's creation to the first frame, then when each step was done.
/// </summary>
public static class Startup
{
    static readonly DateTime Created = Process.GetCurrentProcess().StartTime;
    static readonly string[] Args = Environment.GetCommandLineArgs();
    static readonly List<string> Steps = [];
    static bool shown;

    /// <summary><c>--page general</c>: the page to open, by the id the Slint window uses.</summary>
    public static string Page => Option("--page") ?? "general";

    /// <summary><c>--theme light|dark</c>: overrides the configured colour scheme (for screenshots).</summary>
    public static string? Theme => Option("--theme");

    /// <summary><c>--config path</c>: another config.json than Infodock's.</summary>
    public static string? ConfigPath => Option("--config");

    /// <summary><c>--log path</c>; next to the exe by default.</summary>
    static string LogPath => Option("--log") ?? Path.Combine(AppContext.BaseDirectory, "infodock-settings.log");

    static string? Option(string name)
    {
        var index = Array.IndexOf(Args, name);
        return index >= 0 && index + 1 < Args.Length ? Args[index + 1] : null;
    }

    static long Elapsed() => (long)(DateTime.Now - Created).TotalMilliseconds;

    /// <summary>Notes that a start-up step was done.</summary>
    public static void Step(string name) => Steps.Add($"{name} {Elapsed()}");

    /// <summary>Logs the first frame, once.</summary>
    public static void FirstFrame()
    {
        if (shown)
            return;
        shown = true;
        Log($"settings visible {Elapsed()} ms after the process started ({string.Join(", ", Steps)})");
    }

    public static void Log(string line)
    {
        try
        {
            File.AppendAllText(LogPath, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff} {line}{Environment.NewLine}");
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // A read-only folder: the timings are lost, the window still works.
        }
    }
}
