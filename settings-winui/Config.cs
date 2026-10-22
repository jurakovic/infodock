using System.Text.Json;

namespace InfodockSettings;

/// <summary>
/// The settings in Infodock's <c>config.json</c>, read once at start. Only values that differ
/// from the defaults are stored (SPEC §11.2), so every read names its default. Nothing is
/// written back: the spike shows the values and lets the controls move, it does not save.
/// </summary>
public sealed class Config
{
    readonly JsonElement settings;

    Config(JsonElement settings) => this.settings = settings;

    public static Config Current { get; } = Load(Startup.ConfigPath);

    static Config Load(string? path)
    {
        path ??= Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "app.infodock.desktop",
            "config.json");
        try
        {
            using var document = JsonDocument.Parse(File.ReadAllBytes(path));
            if (document.RootElement.TryGetProperty("settings", out var settings))
                return new Config(settings.Clone());
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            Startup.Log($"config not read, showing the defaults: {e.Message}");
        }
        return new Config(default);
    }

    /// <summary>The value at a dotted path such as <c>background.opacity</c>.</summary>
    JsonElement? Find(string path)
    {
        var element = settings;
        foreach (var part in path.Split('.'))
        {
            if (element.ValueKind != JsonValueKind.Object || !element.TryGetProperty(part, out element))
                return null;
        }
        return element;
    }

    public bool Flag(string path, bool fallback) =>
        Find(path) is { ValueKind: JsonValueKind.True or JsonValueKind.False } value ? value.GetBoolean() : fallback;

    public string? Text(string path) =>
        Find(path) is { ValueKind: JsonValueKind.String } value ? value.GetString() : null;

    public int? Number(string path) =>
        Find(path) is { ValueKind: JsonValueKind.Number } value && value.TryGetInt32(out var number)
            ? number
            : null;

    /// <summary>The index in <paramref name="values"/> of the stored choice, or of <paramref name="fallback"/>.</summary>
    public int Choice(string path, string fallback, params string[] values)
    {
        var index = Array.IndexOf(values, Text(path) ?? fallback);
        return index >= 0 ? index : Math.Max(0, Array.IndexOf(values, fallback));
    }
}
