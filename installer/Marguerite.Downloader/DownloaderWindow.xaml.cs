using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Threading;

namespace MargueriteDownloader
{
    public partial class DownloaderWindow : Window
    {
        private readonly DownloadEngine _engine = new DownloadEngine();
        private readonly DispatcherTimer _watchdog;
        private FilmGate _gate = new FilmGate();
        private CancellationTokenSource _cts;
        private bool _videoReady;
        private bool _videoFailed;
        private bool _launching;
        private bool _audioOn;
        private bool _closing;
        private double _speed;
        private DateTime _startedAt;
        private DateTime _videoStartedAt;
        private string _payloadPath;
        private string _installerPath;
        private string _sourceLabel;

        public DownloaderWindow()
        {
            InitializeComponent();
            FitToWorkArea();

            // If the media stack silently refuses the film (missing codecs are
            // common on Windows N), the gate must still open instead of hanging
            // at 100% forever.
            _watchdog = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
            _watchdog.Tick += Watchdog_Tick;

            Loaded += OnLoaded;
            Closing += OnClosing;
        }

        /// <summary>
        /// Authored at 1040x640 DIPs. Smaller laptops and 150% scaling would
        /// otherwise clip the copy, so the canvas scales down proportionally.
        /// </summary>
        private void FitToWorkArea()
        {
            var workArea = SystemParameters.WorkArea;
            var availableWidth = Math.Max(760.0, workArea.Width - 40.0);
            var availableHeight = Math.Max(480.0, workArea.Height - 40.0);
            var scale = Math.Min(1.0, Math.Min(availableWidth / 1040.0, availableHeight / 640.0));

            RootCanvas.LayoutTransform = new ScaleTransform(scale, scale);
            Width = 1040.0 * scale;
            Height = 640.0 * scale;
        }

        private async void OnLoaded(object sender, RoutedEventArgs e)
        {
            ApplyRoundedCorners();

            DemoBadge.Visibility = DownloaderOptions.Demo ? Visibility.Visible : Visibility.Collapsed;
            _startedAt = DateTime.Now;
            _cts = new CancellationTokenSource();

            StartVideo();

            try
            {
                await RunAsync(_cts.Token);
            }
            catch (OperationCanceledException)
            {
                // The window is already closing; nothing to report.
            }
            catch (Exception ex)
            {
                ShowFailure(ex.Message);
            }
        }

        // =====================================================================
        // Film
        // =====================================================================

        private void StartVideo()
        {
            try
            {
                var path = EnsurePromoVideo();
                Log("film=" + path + " bytes=" + new FileInfo(path).Length);
                VideoSurface.Source = new Uri(path, UriKind.Absolute);
                _videoStartedAt = DateTime.Now;
                _watchdog.Start();

                // LoadedBehavior="Manual" means nothing is opened until Play is
                // called; without this the MediaOpened event never arrives and
                // the window would show the poster forever.
                VideoSurface.Play();
            }
            catch (Exception ex)
            {
                _videoFailed = true;
                FilmNoteText.Text = "片头不可用，已跳过等待";
                Log("film unavailable: " + ex.Message);
                _gate.SkipFilm();
            }
        }

        /// <summary>
        /// Two silent failure modes are possible on Windows: the codec is
        /// missing (no MediaOpened at all) or playback stalls at the end.
        /// Neither may leave the user staring at a finished download.
        /// </summary>
        private void Watchdog_Tick(object sender, EventArgs e)
        {
            if (_videoFailed || _closing)
            {
                _watchdog.Stop();
                return;
            }

            if (!_videoReady)
            {
                if ((DateTime.Now - _videoStartedAt).TotalSeconds >= 10)
                {
                    Log("film never opened within 10s");
                    VideoSurface.Visibility = Visibility.Collapsed;
                    _videoFailed = true;
                    FilmNoteText.Text = "此系统无法播放片头，已跳过等待";
                    _watchdog.Stop();
                    _gate.SkipFilm();
                    TryLaunch();
                }

                return;
            }

            if (_gate.FirstPassComplete)
            {
                _watchdog.Stop();
                return;
            }

            var duration = VideoSurface.NaturalDuration;
            if (!duration.HasTimeSpan)
            {
                return;
            }

            // MediaEnded can be swallowed by some media stacks; a hard cap one
            // second past the natural duration keeps the promise ("one full
            // pass") without risking an endless wait.
            if (VideoSurface.Position >= duration.TimeSpan - TimeSpan.FromSeconds(0.4))
            {
                Log("film end detected by watchdog at " + VideoSurface.Position);
                CompleteFirstPass();
                _watchdog.Stop();
            }
        }

        private static void Log(string message)
        {
            try
            {
                var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(directory);
                File.AppendAllText(
                    Path.Combine(directory, "downloader-media.log"),
                    DateTime.Now.ToString("HH:mm:ss.fff") + "  " + message + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
            }
        }

        private static string EnsurePromoVideo()
        {
            var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
            Directory.CreateDirectory(directory);
            var target = Path.Combine(directory, "promo-0.2.17.mp4");

            using (var source = Assembly.GetExecutingAssembly()
                .GetManifestResourceStream("MargueriteDownloader.Assets.promo.mp4"))
            {
                if (source == null)
                {
                    throw new InvalidOperationException("The embedded film is missing.");
                }

                if (File.Exists(target) && new FileInfo(target).Length == source.Length)
                {
                    return target;
                }

                using (var destination = new FileStream(target, FileMode.Create, FileAccess.Write, FileShare.Read))
                {
                    source.CopyTo(destination);
                }
            }

            return target;
        }

        private void VideoSurface_MediaOpened(object sender, RoutedEventArgs e)
        {
            _videoReady = true;
            Log("media opened duration=" + VideoSurface.NaturalDuration);
            VideoSurface.Position = TimeSpan.Zero;

            VideoSurface.BeginAnimation(
                UIElement.OpacityProperty,
                new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(500))
                {
                    EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
                });

            VideoSurface.Play();
        }

        /// <summary>
        /// The whole rule of this app lives here: one full pass of the film is
        /// mandatory. When it ends and the download already finished, the
        /// installer opens immediately; otherwise the film simply loops.
        /// </summary>
        private void VideoSurface_MediaEnded(object sender, RoutedEventArgs e)
        {
            Log("media ended at " + VideoSurface.Position);
            CompleteFirstPass();

            if (_closing || _launching)
            {
                return;
            }

            // Loop forever while the payload is still downloading.
            VideoSurface.Position = TimeSpan.Zero;
            VideoSurface.Play();
        }

        private void CompleteFirstPass()
        {
            if (_gate.FirstPassComplete)
            {
                return;
            }

            if (_gate.MarkFirstPass())
            {
                StageChip.Text = "即将打开安装程序";
            }
            else
            {
                StatusText.Text = "片头已播完，正在等待下载收尾…";
            }

            TryLaunch();
        }

        private void VideoSurface_MediaFailed(object sender, ExceptionRoutedEventArgs e)
        {
            _videoFailed = true;
            VideoSurface.Visibility = Visibility.Collapsed;
            FilmNoteText.Text = "此系统无法播放片头，已跳过等待";
            Log("media failed: " + (e.ErrorException == null ? "unknown" : e.ErrorException.Message));

            // Without a film the gate cannot be honoured; release it instead of
            // blocking the install behind a broken player.
            _gate.SkipFilm();
            TryLaunch();
        }

        // =====================================================================
        // Download
        // =====================================================================

        private async Task RunAsync(CancellationToken cancellationToken)
        {
            if (DownloaderOptions.Demo)
            {
                await RunDemoAsync(cancellationToken);
                return;
            }

            var work = DownloaderOptions.WorkingDirectory;
            Directory.CreateDirectory(work);

            ReleaseAsset installer;
            if (!string.IsNullOrWhiteSpace(DownloaderOptions.Url))
            {
                installer = new ReleaseAsset
                {
                    Name = Path.GetFileName(new Uri(DownloaderOptions.Url).LocalPath),
                    Url = DownloaderOptions.Url,
                    Size = 0,
                    Sha256 = DownloaderOptions.Sha256
                };
                _sourceLabel = "自定义地址";
            }
            else
            {
                SetPhase("正在连接发布仓库…", 0);
                var release = await _engine.ResolveLatestAsync(DownloaderOptions.LatestReleaseApi, cancellationToken);
                installer = release.Installer;
                _sourceLabel = string.IsNullOrWhiteSpace(release.Tag) ? "GitHub Releases" : release.Tag;
                _installerPath = await EnsureSetupShellAsync(release, work, cancellationToken);
            }

            if (string.IsNullOrWhiteSpace(_installerPath))
            {
                _installerPath = FindLocalSetupShell();
                if (string.IsNullOrWhiteSpace(_installerPath))
                {
                    throw new InvalidOperationException(
                        "找不到安装界面程序 MargueriteSetup.exe，请把它与下载器放在同一目录。");
                }
            }

            var destination = string.IsNullOrWhiteSpace(DownloaderOptions.Output)
                ? Path.Combine(work, installer.Name)
                : Path.GetFullPath(DownloaderOptions.Output);

            SetPhase("正在下载安装包…", 1);
            StatusText.Text = installer.Name + " · " + _sourceLabel;

            var progress = new Progress<DownloadProgressInfo>(OnDownloadProgress);
            await _engine.DownloadAsync(installer.Url, destination, progress, cancellationToken);

            if (!string.IsNullOrWhiteSpace(installer.Sha256))
            {
                SetPhase("正在校验安装包…", 99);
                var expected = NormalizeDigest(installer.Sha256);
                var actual = await _engine.ComputeSha256Async(destination, cancellationToken);
                if (!string.Equals(expected, actual, StringComparison.OrdinalIgnoreCase))
                {
                    TryDelete(destination);
                    throw new InvalidOperationException(
                        "安装包校验失败：下载内容与发布记录不一致。已删除文件，请重试。");
                }
            }

            _payloadPath = destination;
            UpdatePercent(100);
            ProgressBar.Value = 100;

            if (_gate.MarkDownload())
            {
                TryLaunch();
            }
            else
            {
                ShowWaitingForFilm();
            }
        }

        private async Task<string> EnsureSetupShellAsync(
            DownloadEngine.ReleaseInfo release,
            string work,
            CancellationToken cancellationToken)
        {
            if (!string.IsNullOrWhiteSpace(DownloaderOptions.InstallerPath)
                && File.Exists(DownloaderOptions.InstallerPath))
            {
                return Path.GetFullPath(DownloaderOptions.InstallerPath);
            }

            var local = FindLocalSetupShell();
            if (!string.IsNullOrWhiteSpace(local))
            {
                return local;
            }

            if (release.SetupShell == null)
            {
                return null;
            }

            var target = Path.Combine(work, "MargueriteSetup.exe");
            SetPhase("正在准备安装界面…", 0);
            StatusText.Text = "MargueriteSetup.exe";

            var progress = new Progress<DownloadProgressInfo>(info =>
            {
                if (info.Total > 0)
                {
                    UpdatePercent(Math.Min(6.0, 6.0 * info.Received / info.Total));
                }
            });

            await _engine.DownloadAsync(release.SetupShell.Url, target, progress, cancellationToken);
            return target;
        }

        private static string FindLocalSetupShell()
        {
            var beside = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "MargueriteSetup.exe");
            if (File.Exists(beside))
            {
                return beside;
            }

            var work = Path.Combine(DownloaderOptions.WorkingDirectory, "MargueriteSetup.exe");
            if (File.Exists(work))
            {
                return work;
            }

            return null;
        }

        private async Task RunDemoAsync(CancellationToken cancellationToken)
        {
            _sourceLabel = "演示";
            _installerPath = "MargueriteSetup.exe";
            SetPhase("演示模式：模拟下载…", 0);
            StatusText.Text = "Marguerite_0.2.17_x64-setup.exe · 演示";

            const long total = 369904918L;
            for (var i = 0; i <= 100; i++)
            {
                cancellationToken.ThrowIfCancellationRequested();
                OnDownloadProgress(new DownloadProgressInfo
                {
                    Received = total * i / 100,
                    Total = total,
                    BytesPerSecond = total / 9.0
                });
                await Task.Delay(85, cancellationToken);
            }

            UpdatePercent(100);
            ProgressBar.Value = 100;

            if (_gate.MarkDownload())
            {
                TryLaunch();
            }
            else
            {
                ShowWaitingForFilm();
            }
        }

        private void OnDownloadProgress(DownloadProgressInfo info)
        {
            _speed = info.BytesPerSecond;

            var ratio = info.Total > 0 ? (double)info.Received / info.Total : 0;
            var percent = 1.0 + 98.0 * ratio;

            UpdatePercent(percent);
            ProgressBar.Value = Math.Max(0, Math.Min(100, percent));

            var parts = new List<string>();
            parts.Add(info.Total > 0
                ? FormatBytes(info.Received) + " / " + FormatBytes(info.Total)
                : FormatBytes(info.Received));

            if (_speed > 1024)
            {
                parts.Add(FormatBytes((long)_speed) + "/s");
            }

            parts.Add("已用 " + (DateTime.Now - _startedAt).ToString(@"mm\:ss"));
            MetaText.Text = string.Join("  ·  ", parts.ToArray());
        }

        private void UpdatePercent(double percent)
        {
            PercentText.Text = ((int)Math.Round(Math.Max(0, Math.Min(100, percent))))
                .ToString(CultureInfo.InvariantCulture);
        }

        private void SetPhase(string status, double percent)
        {
            StatusText.Text = status;
            StageChip.Text = percent >= 99 ? "校验中" : percent > 0 ? "下载中" : "准备中";
            if (percent > 0)
            {
                UpdatePercent(percent);
            }
        }

        private void ShowWaitingForFilm()
        {
            TitleText.Text = "下载完成";
            StatusText.Text = "片头还在播放。播完这一遍，就打开安装界面。";
            StageChip.Text = "等待片头";
            FilmNoteText.Text = "这是刻意的：先看完这一遍";
        }

        // =====================================================================
        // Hand-off
        // =====================================================================

        private async void TryLaunch()
        {
            if (_launching || !_gate.ShouldLaunch)
            {
                return;
            }

            _launching = true;
            TitleText.Text = "即将打开安装界面";
            StageChip.Text = "启动中";
            StatusText.Text = "正在启动安装程序…";
            ProgressBar.Value = 100;
            UpdatePercent(100);

            await Task.Delay(520);

            if (DownloaderOptions.Demo || DownloaderOptions.NoLaunch)
            {
                TitleText.Text = DownloaderOptions.Demo ? "演示结束" : "下载完成";
                StatusText.Text = "本次运行不会打开安装程序。";
                StageChip.Text = "完成";
                CancelButton.Content = "关闭";
                FolderButton.Visibility = Visibility.Visible;
                _launching = false;
                return;
            }

            try
            {
                if (string.IsNullOrWhiteSpace(_installerPath) || string.IsNullOrWhiteSpace(_payloadPath))
                {
                    throw new InvalidOperationException("安装界面或安装包缺失。");
                }

                var startInfo = new ProcessStartInfo
                {
                    FileName = _installerPath,
                    Arguments = "--payload \"" + _payloadPath + "\"",
                    UseShellExecute = false,
                    WorkingDirectory = Path.GetDirectoryName(_installerPath) ?? AppDomain.CurrentDomain.BaseDirectory
                };

                Process.Start(startInfo);
                _closing = true;
                Close();
            }
            catch (Exception ex)
            {
                _launching = false;
                ShowFailure("无法打开安装界面：" + ex.Message);
            }
        }

        private void ShowFailure(string message)
        {
            TitleText.Text = "下载没有完成";
            StatusText.Text = message;
            StageChip.Text = "已中断";
            RetryButton.Visibility = Visibility.Visible;
            FolderButton.Visibility = Visibility.Visible;
            CancelButton.Content = "关闭";
        }

        private void Retry_Click(object sender, RoutedEventArgs e)
        {
            RetryButton.Visibility = Visibility.Collapsed;
            FolderButton.Visibility = Visibility.Collapsed;
            CancelButton.Content = "取消";
            TitleText.Text = "正在下载";
            _gate = new FilmGate();
            _launching = false;
            _startedAt = DateTime.Now;
            _cts = new CancellationTokenSource();

            var token = _cts.Token;
            RunAsync(token).ContinueWith(
                task =>
                {
                    if (task.IsFaulted && task.Exception != null)
                    {
                        Dispatcher.Invoke(() => ShowFailure(task.Exception.GetBaseException().Message));
                    }
                },
                CancellationToken.None,
                TaskContinuationOptions.OnlyOnFaulted,
                TaskScheduler.FromCurrentSynchronizationContext());
        }

        private void Folder_Click(object sender, RoutedEventArgs e)
        {
            var directory = !string.IsNullOrWhiteSpace(_payloadPath)
                ? Path.GetDirectoryName(_payloadPath)
                : DownloaderOptions.WorkingDirectory;

            try
            {
                if (!string.IsNullOrEmpty(directory) && Directory.Exists(directory))
                {
                    Process.Start(new ProcessStartInfo
                    {
                        FileName = "explorer.exe",
                        Arguments = "\"" + directory + "\"",
                        UseShellExecute = true
                    });
                }
            }
            catch
            {
            }
        }

        private void Cancel_Click(object sender, RoutedEventArgs e)
        {
            Close();
        }

        private void OnClosing(object sender, CancelEventArgs e)
        {
            if (_closing)
            {
                return;
            }

            _closing = true;
            _watchdog.Stop();
            try
            {
                _cts.Cancel();
            }
            catch
            {
            }

            try
            {
                VideoSurface.Stop();
                VideoSurface.Close();
            }
            catch
            {
            }
        }

        // =====================================================================
        // Window chrome
        // =====================================================================

        private void TopBar_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
        {
            if (e.ButtonState != MouseButtonState.Pressed)
            {
                return;
            }

            try
            {
                DragMove();
            }
            catch (InvalidOperationException)
            {
            }
        }

        private void Minimize_Click(object sender, RoutedEventArgs e)
        {
            WindowState = WindowState.Minimized;
        }

        private void Close_Click(object sender, RoutedEventArgs e)
        {
            Close();
        }

        private void Sound_Click(object sender, RoutedEventArgs e)
        {
            _audioOn = !_audioOn;
            VideoSurface.IsMuted = !_audioOn;
            VideoSurface.Volume = _audioOn ? 0.3 : 0;
            SoundGlyph.Text = _audioOn ? "\uE767" : "\uE74F";
            SoundButton.ToolTip = _audioOn ? "静音" : "开启声音";

            if (_audioOn && _videoReady && !_videoFailed)
            {
                VideoSurface.Play();
            }
        }

        private void ApplyRoundedCorners()
        {
            try
            {
                var handle = new WindowInteropHelper(this).Handle;
                if (handle == IntPtr.Zero)
                {
                    return;
                }

                var preference = 2;
                DwmSetWindowAttribute(handle, 33, ref preference, sizeof(int));
            }
            catch
            {
            }
        }

        // =====================================================================
        // Helpers
        // =====================================================================

        private static string NormalizeDigest(string digest)
        {
            if (string.IsNullOrWhiteSpace(digest))
            {
                return string.Empty;
            }

            var trimmed = digest.Trim();
            var colon = trimmed.IndexOf(':');
            return colon >= 0 ? trimmed.Substring(colon + 1).Trim() : trimmed;
        }

        private static string FormatBytes(long bytes)
        {
            if (bytes < 1024)
            {
                return bytes.ToString(CultureInfo.InvariantCulture) + " B";
            }

            string[] units = { "KB", "MB", "GB" };
            var value = bytes / 1024.0;
            var unit = 0;
            while (value >= 1024 && unit < units.Length - 1)
            {
                value /= 1024;
                unit++;
            }

            var format = value >= 100 ? "0" : value >= 10 ? "0.0" : "0.00";
            return value.ToString(format, CultureInfo.InvariantCulture) + " " + units[unit];
        }

        private static void TryDelete(string path)
        {
            try
            {
                if (!string.IsNullOrWhiteSpace(path) && File.Exists(path))
                {
                    File.Delete(path);
                }
            }
            catch
            {
            }
        }

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attribute, ref int value, int size);
    }
}
