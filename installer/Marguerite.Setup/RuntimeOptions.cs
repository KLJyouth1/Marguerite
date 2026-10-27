using System;
using System.IO;

namespace MargueriteSetup
{
    /// <summary>
    /// Command-line switches used by the installer shell.
    /// They are intentionally undocumented in the UI: demo/preview exist so the
    /// visual states can be exercised without touching a real installation.
    /// </summary>
    public static class RuntimeOptions
    {
        public static bool Demo { get; private set; }

        /// <summary>Headless check of payload discovery, hashing and argument construction.</summary>
        public static bool SelfTest { get; private set; }

        public static string Preview { get; private set; }

        public static string PayloadOverride { get; private set; }

        /// <summary>Force a UI language for screenshots ("zh" or "en").</summary>
        public static string Language { get; private set; }

        public static void Parse(string[] args)
        {
            if (args == null)
            {
                return;
            }

            for (var i = 0; i < args.Length; i++)
            {
                var arg = args[i] ?? string.Empty;

                if (string.Equals(arg, "--demo", StringComparison.OrdinalIgnoreCase))
                {
                    Demo = true;
                    continue;
                }

                if (string.Equals(arg, "--selftest", StringComparison.OrdinalIgnoreCase))
                {
                    SelfTest = true;
                    continue;
                }

                if (arg.StartsWith("--preview=", StringComparison.OrdinalIgnoreCase))
                {
                    Preview = arg.Substring("--preview=".Length).Trim().ToLowerInvariant();
                    continue;
                }

                if (string.Equals(arg, "--preview", StringComparison.OrdinalIgnoreCase) && i + 1 < args.Length)
                {
                    Preview = (args[++i] ?? string.Empty).Trim().ToLowerInvariant();
                    continue;
                }

                if (arg.StartsWith("--payload=", StringComparison.OrdinalIgnoreCase))
                {
                    PayloadOverride = arg.Substring("--payload=".Length).Trim().Trim('"');
                    continue;
                }

                if (arg.StartsWith("--lang=", StringComparison.OrdinalIgnoreCase))
                {
                    Language = arg.Substring("--lang=".Length).Trim().ToLowerInvariant();
                    continue;
                }

                if (string.Equals(arg, "--payload", StringComparison.OrdinalIgnoreCase) && i + 1 < args.Length)
                {
                    PayloadOverride = (args[++i] ?? string.Empty).Trim().Trim('"');
                }
            }

            if (!string.IsNullOrWhiteSpace(PayloadOverride) && !File.Exists(PayloadOverride))
            {
                PayloadOverride = null;
            }
        }
    }
}
