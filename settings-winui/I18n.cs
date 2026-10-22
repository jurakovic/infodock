using System.Globalization;
using System.Text.Json;
using Microsoft.UI.Xaml.Markup;

namespace InfodockSettings;

/// <summary>Infodock's message catalog (<c>i18n/en.json</c>, embedded), the one the Slint window reads.</summary>
public static class I18n
{
    static readonly Dictionary<string, string> Messages = Load();

    static Dictionary<string, string> Load()
    {
        var messages = new Dictionary<string, string>();
        using var stream = typeof(I18n).Assembly.GetManifestResourceStream("en.json");
        if (stream is null)
            return messages;
        using var document = JsonDocument.Parse(stream);
        // Plural messages are objects; Settings' pages use none of them.
        foreach (var message in document.RootElement.EnumerateObject())
        {
            if (message.Value.ValueKind == JsonValueKind.String)
                messages[message.Name] = message.Value.GetString() ?? "";
        }
        return messages;
    }

    /// <summary>The message <paramref name="key"/>, or the key itself when the catalog lacks it.</summary>
    public static string T(string key) => Messages.TryGetValue(key, out var text) ? text : key;

    /// <summary>The message with its <c>{name}</c> placeholder filled.</summary>
    public static string T(string key, string name, object value) =>
        T(key).Replace("{" + name + "}", Convert.ToString(value, CultureInfo.CurrentCulture));
}

/// <summary>A message in XAML: <c>{local:Tr Key=settings.title}</c>.</summary>
[MarkupExtensionReturnType(ReturnType = typeof(string))]
public sealed partial class Tr : MarkupExtension
{
    public string Key { get; set; } = "";

    protected override object ProvideValue() => I18n.T(Key);
}
