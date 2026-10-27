using System;
using System.IO;

namespace MargueriteDownloader
{
    /// <summary>
    /// Switches for the downloader. The defaults are the shipped behaviour;
    /// the test switches exist so the gating logic can be proven without
    /// touching a real release.
    /// </summary>
    public static class DownloaderOptions
    {
        public const string DownloaderUrl =
            "https://github.com/KLJyouth1/Marguerite-Releases/releases/latest/download/MargueriteDownloader.exe";

        public const string SetupUrl =
            "https://github.com/KLJyouth1/Marguerite-Releases/releases/latest/download/MargueriteSetup.exe";

        public const string LatestReleaseApi =
            "https://api.github.com/repos/KLJyouth1/Marguerite-Releases/releases/latest";

        public static bool Demo { get; private set; }

        public static bool SelfTest { get; private set; }

        public static bool NoLaunch { get; private set; }

        public static string Url { get; private set; }

        public static string Output { get; private set; }

        public static string Sha256 { get; private set; }

        public static string InstallerPath { get; private set; }

        public static void Parse(string[] args)
        {
            if (args == null)
            {
                return;
            }

            for (var i = 0; i < args.Length; i++)
            {
                var arg = args[i] ?? string.Empty;

                if (Equal(arg, "--demo"))
                {
                    Demo = true;
                    continue;
                }

                if (Equal(arg, "--selftest"))
                {
                    SelfTest = true;
                    continue;
                }

                if (Equal(arg, "--no-launch"))
                {
                    NoLaunch = true;
                    continue;
                }

                if (Starts(arg, "--url="))
                {
                    Url = Value(arg, "--url=");
                    continue;
                }

                if (Equal(arg, "--url") && i + 1 < args.Length)
                {
                    Url = (args[++i] ?? string.Empty).Trim().Trim('"');
                    continue;
                }

                if (Starts(arg, "--out="))
                {
                    Output = Value(arg, "--out=");
                    continue;
                }

                if (Equal(arg, "--out") && i + 1 < args.Length)
                {
                    Output = (args[++i] ?? string.Empty).Trim().Trim('"');
                    continue;
                }

                if (Starts(arg, "--sha256="))
                {
                    Sha256 = Value(arg, "--sha256=");
                    continue;
                }

                if (Equal(arg, "--sha256") && i + 1 < args.Length)
                {
                    Sha256 = (args[++i] ?? string.Empty).Trim().Trim('"');
                    continue;
                }

                if (Starts(arg, "--installer="))
                {
                    InstallerPath = Value(arg, "--installer=");
                    continue;
                }

                if (Equal(arg, "--installer") && i + 1 < args.Length)
                {
                    InstallerPath = (args[++i] ?? string.Empty).Trim().Trim('"');
                }
            }
        }

        private static bool Equal(string arg, string name)
        {
            return string.Equals(arg, name, StringComparison.OrdinalIgnoreCase);
        }

        private static bool Starts(string arg, string prefix)
        {
            return arg.StartsWith(prefix, StringComparison.OrdinalIgnoreCase);
        }

        private static string Value(string arg, string prefix)
        {
            return arg.Substring(prefix.Length).Trim().Trim('"');
        }

        /// <summary>%LOCALAPPDATA%\Marguerite\downloader — writable without elevation.</summary>
        public static string WorkingDirectory
        {
            get
            {
                return Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "Marguerite",
                    "downloader");
            }
        }
    }
}
