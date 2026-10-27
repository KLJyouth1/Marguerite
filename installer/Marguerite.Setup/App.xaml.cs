using System;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Threading;

namespace MargueriteSetup
{
    public partial class App : Application
    {
        protected override void OnStartup(StartupEventArgs e)
        {
            RuntimeOptions.Parse(e.Args);

            if (RuntimeOptions.SelfTest)
            {
                Shutdown(SelfTest.Run());
                return;
            }

            DispatcherUnhandledException += OnDispatcherUnhandledException;

            base.OnStartup(e);
        }

        private static void OnDispatcherUnhandledException(object sender, DispatcherUnhandledExceptionEventArgs e)
        {
            try
            {
                var dir = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(dir);
                var path = Path.Combine(dir, "setup-crash.log");
                File.AppendAllText(
                    path,
                    DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + "  " + e.Exception + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
                // A crash logger must never mask the original failure.
            }
        }
    }
}
