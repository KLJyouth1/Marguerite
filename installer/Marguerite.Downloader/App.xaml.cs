using System;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Threading;

namespace MargueriteDownloader
{
    public partial class App : Application
    {
        protected override void OnStartup(StartupEventArgs e)
        {
            DownloaderOptions.Parse(e.Args);

            if (DownloaderOptions.SelfTest)
            {
                Shutdown(SelfTest.Run());
                return;
            }

            DispatcherUnhandledException += OnUnhandled;

            base.OnStartup(e);
        }

        private static void OnUnhandled(object sender, DispatcherUnhandledExceptionEventArgs e)
        {
            try
            {
                var dir = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(dir);
                File.AppendAllText(
                    Path.Combine(dir, "downloader-crash.log"),
                    DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + "  " + e.Exception + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
            }
        }
    }
}
