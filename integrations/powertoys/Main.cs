// PowerToys Run plugin for Kor (Master Build Prompt §27.10, ADR 0052).
// Typing `kor <soru>` shows "Kor'a sor" (opens kor://task?prompt=…, the desktop app puts it in
// the composer), recent projects ("Kor'da aç", kor://open?path=…), recent sessions
// (kor://resume?id=…) and waiting approvals. All data and labels come from
// `kor os-integration feed` (JSON, already localized); the kor path is written next to this DLL
// at install time (kor-path.txt).

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using Wox.Plugin;

namespace Community.PowerToys.Run.Plugin.Kor;

public sealed class Main : IPlugin
{
    public static string PluginID => "6B6F7201000040008000000000000001";

    private const int TimeoutMs = 3000;
    private const string Icon = "Images\\kor.dark.png";

    private string _kor = "kor";
    private string _cachedQuery = "\0";
    private DateTime _cachedAt = DateTime.MinValue;
    private JsonDocument? _cached;

    public string Name => "Kor";

    public string Description => "Kor coding agent: ask, open projects and sessions";

    public void Init(PluginInitContext context)
    {
        var dir = Path.GetDirectoryName(typeof(Main).Assembly.Location) ?? ".";
        var file = Path.Combine(dir, "kor-path.txt");
        if (File.Exists(file))
        {
            var path = File.ReadAllText(file).Trim();
            if (path.Length > 0 && File.Exists(path))
            {
                _kor = path;
            }
        }
    }

    public List<Result> Query(Query query)
    {
        var search = (query.Search ?? string.Empty).Trim();
        var results = new List<Result>();
        var feed = Feed(search);
        if (feed is null)
        {
            return results;
        }

        var root = feed.RootElement;
        var labels = root.GetProperty("labels");
        if (root.TryGetProperty("ask", out var ask) && ask.ValueKind == JsonValueKind.String)
        {
            results.Add(Item(root.GetProperty("askLabel").GetString() ?? search, "Kor", ask.GetString()!, 100));
        }
        else
        {
            results.Add(Item(Text(labels, "newTask"), "Kor", root.GetProperty("newTask").GetString()!, 100));
        }

        foreach (var p in root.GetProperty("projects").EnumerateArray())
        {
            results.Add(Item(p.GetProperty("name").GetString() ?? string.Empty, $"{Text(labels, "open")} · {p.GetProperty("path").GetString()}", p.GetProperty("url").GetString()!, 80));
        }

        foreach (var s in root.GetProperty("sessions").EnumerateArray())
        {
            results.Add(Item(s.GetProperty("title").GetString() ?? string.Empty, Text(labels, "sessions"), s.GetProperty("url").GetString()!, 60));
        }

        foreach (var w in root.GetProperty("waiting").EnumerateArray())
        {
            var id = w.GetProperty("id").GetString() ?? string.Empty;
            results.Add(new Result
            {
                Title = $"{Text(labels, "approve")}: {w.GetProperty("summary").GetString()}",
                SubTitle = Text(labels, "waiting"),
                IcoPath = Icon,
                Score = 90,
                Action = _ =>
                {
                    Run(new[] { "os-integration", "approve", "--id", id });
                    return true;
                },
            });
        }

        return results;
    }

    private static string Text(JsonElement labels, string key) =>
        labels.TryGetProperty(key, out var v) ? v.GetString() ?? key : key;

    private static Result Item(string title, string subtitle, string url, int score) => new()
    {
        Title = title,
        SubTitle = subtitle,
        IcoPath = Icon,
        Score = score,
        Action = _ => Open(url),
    };

    private static bool Open(string url)
    {
        // Only Kor's own links are opened.
        if (!url.StartsWith("kor://", StringComparison.Ordinal))
        {
            return false;
        }

        try
        {
            Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });
            return true;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return false;
        }
    }

    private JsonDocument? Feed(string search)
    {
        // PowerToys asks on every keystroke: reuse the last answer for the same text for 2 s.
        if (_cached is not null && search == _cachedQuery && DateTime.UtcNow - _cachedAt < TimeSpan.FromSeconds(2))
        {
            return _cached;
        }

        var args = new List<string> { "os-integration", "feed", "--limit", "5" };
        if (search.Length > 0)
        {
            args.Add("--query");
            args.Add(search);
        }

        var output = Run(args);
        if (output is null)
        {
            return null;
        }

        try
        {
            var doc = JsonDocument.Parse(output);
            _cached?.Dispose();
            _cached = doc;
            _cachedQuery = search;
            _cachedAt = DateTime.UtcNow;
            return doc;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private string? Run(IEnumerable<string> args)
    {
        var info = new ProcessStartInfo(_kor)
        {
            CreateNoWindow = true,
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var a in args)
        {
            info.ArgumentList.Add(a);
        }

        try
        {
            using var p = Process.Start(info);
            if (p is null)
            {
                return null;
            }

            var read = p.StandardOutput.ReadToEndAsync();
            if (!p.WaitForExit(TimeoutMs))
            {
                p.Kill(true);
                return null;
            }

            return p.ExitCode == 0 ? read.Result : null;
        }
        catch (Exception e) when (e is System.ComponentModel.Win32Exception or InvalidOperationException or IOException)
        {
            return null;
        }
    }
}
