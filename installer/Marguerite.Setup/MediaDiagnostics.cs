using System;
using System.IO;
using System.Text;

namespace MargueriteSetup
{
    /// <summary>
    /// One-line diagnostics for the embedded film. MediaElement can fail
    /// silently, so the log is the only way to tell "poster shown because the
    /// codec is missing" from "video is actually playing".
    /// </summary>
    internal static class MediaDiagnostics
    {
        public static void Log(string message)
        {
            try
            {
                var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(directory);
                File.AppendAllText(
                    Path.Combine(directory, "setup-media.log"),
                    DateTime.Now.ToString("HH:mm:ss.fff") + "  " + message + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
            }
        }
    }
}
