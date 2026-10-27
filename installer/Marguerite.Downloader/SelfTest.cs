using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace MargueriteDownloader
{
    /// <summary>
    /// Headless proof of the downloader contract. No window, no network, no
    /// installer is started: it exercises the film gate and the CLI switches,
    /// then writes the evidence to %TEMP%\MargueriteSetup\downloader-selftest.log.
    /// </summary>
    public static class SelfTest
    {
        public static int Run()
        {
            var lines = new List<string>();
            var ok = true;

            void Check(string name, bool condition, string detail)
            {
                lines.Add(string.Format("{0} {1}{2}", condition ? "PASS" : "FAIL", name,
                    string.IsNullOrEmpty(detail) ? string.Empty : "  (" + detail + ")"));
                ok &= condition;
            }

            // Case A: the payload arrives long before the film's first pass ends.
            var gate = new FilmGate();
            Check("download first: no launch", !gate.MarkDownload(), "downloadComplete=" + gate.DownloadComplete);
            Check("download first: still no launch", !gate.ShouldLaunch, "firstPass=" + gate.FirstPassComplete);
            Check("download first: film ends -> launch", gate.MarkFirstPass(), "shouldLaunch=" + gate.ShouldLaunch);

            // Case B: the film ends first and the download is still running.
            gate = new FilmGate();
            Check("film first: no launch", !gate.MarkFirstPass(), "firstPass=" + gate.FirstPassComplete);
            Check("film first: download ends -> launch", gate.MarkDownload(), "shouldLaunch=" + gate.ShouldLaunch);

            // Case C: a broken player must not block installation forever.
            gate = new FilmGate();
            gate.SkipFilm();
            Check("no film: download still gates", !gate.ShouldLaunch, "downloadComplete=" + gate.DownloadComplete);
            Check("no film: download ends -> launch", gate.MarkDownload(), "shouldLaunch=" + gate.ShouldLaunch);

            // CLI switches.
            DownloaderOptions.Parse(new[] { "--url", "https://example.com/a.exe", "--out", @"C:\tmp\a.exe", "--no-launch", "--sha256", "abc" });
            Check("parse --url", DownloaderOptions.Url == "https://example.com/a.exe", DownloaderOptions.Url);
            Check("parse --out", DownloaderOptions.Output == @"C:\tmp\a.exe", DownloaderOptions.Output);
            Check("parse --no-launch", DownloaderOptions.NoLaunch, null);
            Check("parse --sha256", DownloaderOptions.Sha256 == "abc", DownloaderOptions.Sha256);

            DownloaderOptions.Parse(new[] { "--url=https://example.com/b.exe" });
            Check("parse --url= form", DownloaderOptions.Url == "https://example.com/b.exe", DownloaderOptions.Url);

            // Working directory must stay inside the per-user profile.
            var work = DownloaderOptions.WorkingDirectory;
            var expectedRoot = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "Marguerite",
                "downloader");
            Check("working directory", string.Equals(work, expectedRoot, StringComparison.OrdinalIgnoreCase), work);

            lines.Add("RESULT=" + (ok ? "PASS" : "FAIL"));

            try
            {
                var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(directory);
                File.WriteAllText(
                    Path.Combine(directory, "downloader-selftest.log"),
                    string.Join(Environment.NewLine, lines.ToArray()) + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
            }

            return ok ? 0 : 1;
        }
    }
}
