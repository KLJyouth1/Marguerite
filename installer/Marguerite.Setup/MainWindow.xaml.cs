using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using WinForms = System.Windows.Forms;

namespace MargueriteSetup
{
    public partial class MainWindow : Window
    {
        private enum Step
        {
            Welcome,
            License,
            Options,
            Installing,
            Finish,
            Error
        }

        private readonly Localization _loc = new Localization();
        private readonly DispatcherTimer _loopTimer;
        private readonly DispatcherTimer _elapsedTimer;
        private readonly SemaphoreSlim _sceneGate = new SemaphoreSlim(1, 1);

        private Step _currentStep = Step.Welcome;
        private PayloadInfo _payload;
        private string _installedDir;
        private bool _videoReady;
        private bool _videoFailed;
        private bool _reducedMotion;
        private bool _sceneTransitioning;
        private bool _audioOn;
        private bool _isInstalling;
        private bool _allowClose;
        private bool _closePromptOpen;
        private CancellationTokenSource _installCts;
        private InstallerRunner _runner;
        private DateTime _installStart;
        private double _percent;
        private InstallStage _lastStage = InstallStage.Verifying;
        private InstallResult _lastError;

        private static readonly Brush DotDoneBrush = new SolidColorBrush(Color.FromArgb(0x66, 0xA9, 0xD3, 0xF2));
        private static readonly Brush DotIdleBrush = new SolidColorBrush(Color.FromArgb(0x33, 0xFF, 0xFF, 0xFF));

        static MainWindow()
        {
            DotDoneBrush.Freeze();
            DotIdleBrush.Freeze();
        }

        public MainWindow()
        {
            InitializeComponent();
            DataContext = this;

            FitToWorkArea();

            _reducedMotion = !SystemParameters.ClientAreaAnimation || SystemParameters.HighContrast;

            _loopTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
            _loopTimer.Tick += LoopTimer_Tick;

            _elapsedTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
            _elapsedTimer.Tick += ElapsedTimer_Tick;

            Loaded += MainWindow_Loaded;
            Closing += MainWindow_Closing;
            PreviewKeyDown += MainWindow_PreviewKeyDown;
        }

        public Localization L
        {
            get { return _loc; }
        }

        private static string DefaultInstallDir
        {
            get
            {
                return Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "Marguerite");
            }
        }

        /// <summary>
        /// The composition is authored at 1200x720 DIPs. On 1366x768 laptops or
        /// at 150% scaling that budget no longer fits the work area, so the
        /// canvas is scaled down proportionally instead of being clipped by
        /// the taskbar.
        /// </summary>
        private void FitToWorkArea()
        {
            var workArea = SystemParameters.WorkArea;
            var availableWidth = Math.Max(800.0, workArea.Width - 40.0);
            var availableHeight = Math.Max(520.0, workArea.Height - 40.0);
            var scale = Math.Min(1.0, Math.Min(availableWidth / 1200.0, availableHeight / 720.0));

            RootCanvas.LayoutTransform = new ScaleTransform(scale, scale);
            Width = 1200.0 * scale;
            Height = 720.0 * scale;
        }

        // =====================================================================
        // Startup
        // =====================================================================

        private async void MainWindow_Loaded(object sender, RoutedEventArgs e)
        {
            ApplyRoundedCorners();

            DemoBadge.Visibility = RuntimeOptions.Demo ? Visibility.Visible : Visibility.Collapsed;
            LicenseText.Text = LoadTextResource("LICENSE.txt");
            InstallPathBox.Text = DefaultInstallDir;
            _payload = ResolvePayload();
            UpdatePayloadStatus();
            UpdateSoundUi();
            LanguageButtonText.Text = _loc.LanguageButton;
            UpdateStepper(Step.Welcome);

            if (!string.IsNullOrWhiteSpace(RuntimeOptions.Preview))
            {
                ApplyPreview();
            }
            else
            {
                ShowStep(Step.Welcome, false);
            }

            await PrepareVideoAsync();
            await GoToSceneAsync(_currentStep, false);

            if (string.IsNullOrWhiteSpace(RuntimeOptions.Preview))
            {
                WelcomeStartButton.Focus();
            }

            _loopTimer.Start();
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

                // DWMWA_WINDOW_CORNER_PREFERENCE = 33, DWMWCP_ROUND = 2.
                // Windows 10 simply rejects it and keeps its square frame.
                var preference = 2;
                DwmSetWindowAttribute(handle, 33, ref preference, sizeof(int));
            }
            catch
            {
            }
        }

        private void ApplyPreview()
        {
            switch ((RuntimeOptions.Preview ?? string.Empty).ToLowerInvariant())
            {
                case "license":
                    LicenseAgreeToggle.IsChecked = true;
                    ShowStep(Step.License, false);
                    break;

                case "options":
                    ShowStep(Step.Options, false);
                    break;

                case "installing":
                    ShowStep(Step.Installing, false);
                    _percent = 62;
                    PercentText.Text = "62";
                    ProgressBar.Value = 62;
                    _lastStage = InstallStage.Writing;
                    StageText.Text = L["StageWriting"];
                    ElapsedText.Text = string.Format(CultureInfo.CurrentCulture, L["ElapsedFormat"], "00:12");
                    CancelInstallButton.IsEnabled = true;
                    break;

                case "finish":
                    _installedDir = DefaultInstallDir;
                    ShowFinish();
                    break;

                case "error":
                    ShowError(new InstallResult
                    {
                        ErrorCode = "exit_code",
                        ExitCode = 1603,
                        LogPath = Path.Combine(Path.GetTempPath(), "MargueriteSetup", "setup-preview.log")
                    });
                    break;

                default:
                    ShowStep(Step.Welcome, false);
                    break;
            }
        }

        private async Task PrepareVideoAsync()
        {
            try
            {
                var path = await Task.Run(() => EnsurePromoVideo());
                VideoSurface.Source = new Uri(path, UriKind.Absolute);

                // LoadedBehavior="Manual": the media is only opened once Play()
                // is called. Without this the MediaOpened handler never runs and
                // every screen silently falls back to its poster frame.
                VideoSurface.Play();
            }
            catch (Exception ex)
            {
                _videoFailed = true;
                VideoSurface.Visibility = Visibility.Collapsed;
                Debug.WriteLine("Video unavailable: " + ex);
            }
        }

        private static string EnsurePromoVideo()
        {
            var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
            Directory.CreateDirectory(directory);
            // 文件名带版本号只为避免缓存复用；版本取内置 pin，不写死。
            var target = Path.Combine(directory, "promo-" + InstallerRunner.ExpectedVersion + ".mp4");

            var assembly = Assembly.GetExecutingAssembly();
            using (var source = assembly.GetManifestResourceStream("MargueriteSetup.Assets.promo.mp4"))
            {
                if (source == null)
                {
                    throw new InvalidOperationException("Embedded promo video is missing.");
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

        private static string LoadTextResource(string name)
        {
            try
            {
                var uri = new Uri("pack://application:,,,/Assets/" + name);
                var resource = System.Windows.Application.GetResourceStream(uri);
                if (resource == null)
                {
                    return string.Empty;
                }

                using (var reader = new StreamReader(resource.Stream, Encoding.UTF8))
                {
                    return reader.ReadToEnd();
                }
            }
            catch
            {
                return string.Empty;
            }
        }

        // =====================================================================
        // Video scenes
        // =====================================================================

        private struct SceneRange
        {
            public readonly double Start;
            public readonly double End;

            public SceneRange(double start, double end)
            {
                Start = start;
                End = end;
            }
        }

        private static SceneRange SceneFor(Step step)
        {
            switch (step)
            {
                case Step.Welcome:
                    return new SceneRange(78, 90);
                case Step.License:
                    return new SceneRange(68, 78);
                case Step.Options:
                    return new SceneRange(32, 42);
                case Step.Installing:
                    return new SceneRange(44, 62);
                case Step.Finish:
                    return new SceneRange(84, 90);
                case Step.Error:
                    return new SceneRange(44, 56);
                default:
                    return new SceneRange(78, 90);
            }
        }

        private static string PosterFor(Step step)
        {
            switch (step)
            {
                case Step.Welcome:
                    return "poster-welcome.jpg";
                case Step.License:
                    return "poster-license.jpg";
                case Step.Options:
                    return "poster-options.jpg";
                case Step.Installing:
                    return "poster-install.jpg";
                case Step.Finish:
                    return "poster-finish.jpg";
                default:
                    return "poster-install.jpg";
            }
        }

        private void SetPosterForStep(Step step)
        {
            try
            {
                PosterSurface.Source = new BitmapImage(new Uri("pack://application:,,,/Assets/" + PosterFor(step)));
            }
            catch
            {
            }
        }

        private async Task GoToSceneAsync(Step step, bool animate)
        {
            if (!_videoReady || _videoFailed || _reducedMotion)
            {
                SetPosterForStep(step);
                return;
            }

            await _sceneGate.WaitAsync();
            try
            {
                _sceneTransitioning = true;
                var scene = SceneFor(step);

                if (animate)
                {
                    await FadeOverlayAsync(0.96, 170);
                    VideoSurface.Position = TimeSpan.FromSeconds(scene.Start);
                    VideoSurface.Play();
                    await Task.Delay(80);
                    await FadeOverlayAsync(0, 420);
                }
                else
                {
                    VideoSurface.Position = TimeSpan.FromSeconds(scene.Start);
                    VideoSurface.Play();
                    SceneFade.Opacity = 0;
                }
            }
            catch (Exception ex)
            {
                Debug.WriteLine("Scene transition failed: " + ex);
                SetPosterForStep(step);
            }
            finally
            {
                _sceneTransitioning = false;
                _sceneGate.Release();
            }
        }

        private Task FadeOverlayAsync(double to, int milliseconds)
        {
            var completion = new TaskCompletionSource<bool>();
            var animation = new DoubleAnimation(to, TimeSpan.FromMilliseconds(milliseconds))
            {
                EasingFunction = new CubicEase { EasingMode = EasingMode.EaseInOut }
            };

            animation.Completed += (sender, args) =>
            {
                SceneFade.Opacity = to;
                SceneFade.BeginAnimation(UIElement.OpacityProperty, null);
                completion.TrySetResult(true);
            };

            SceneFade.BeginAnimation(UIElement.OpacityProperty, animation);
            return completion.Task;
        }

        private void VideoSurface_MediaOpened(object sender, RoutedEventArgs e)
        {
            _videoReady = true;
            MediaDiagnostics.Log("opened duration=" + VideoSurface.NaturalDuration);

            if (_reducedMotion)
            {
                VideoSurface.Pause();
                VideoSurface.Opacity = 0;
                SetPosterForStep(_currentStep);
                return;
            }

            var scene = SceneFor(_currentStep);
            VideoSurface.Position = TimeSpan.FromSeconds(scene.Start);
            VideoSurface.BeginAnimation(
                UIElement.OpacityProperty,
                new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(420))
                {
                    EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
                });
            VideoSurface.Play();
        }

        private void VideoSurface_MediaFailed(object sender, ExceptionRoutedEventArgs e)
        {
            _videoFailed = true;
            VideoSurface.Visibility = Visibility.Collapsed;
            SetPosterForStep(_currentStep);
            MediaDiagnostics.Log("failed: " + (e.ErrorException == null ? "unknown" : e.ErrorException.Message));
            Debug.WriteLine("Media failed: " + e.ErrorException);
        }

        private void VideoSurface_MediaEnded(object sender, RoutedEventArgs e)
        {
            if (_reducedMotion || _videoFailed)
            {
                return;
            }

            VideoSurface.Position = TimeSpan.FromSeconds(SceneFor(_currentStep).Start);
            VideoSurface.Play();
        }

        private void LoopTimer_Tick(object sender, EventArgs e)
        {
            if (!_videoReady || _videoFailed || _reducedMotion || _sceneTransitioning)
            {
                return;
            }

            var scene = SceneFor(_currentStep);
            var position = VideoSurface.Position.TotalSeconds;
            if (position >= scene.End || position < scene.Start - 0.75)
            {
                VideoSurface.Position = TimeSpan.FromSeconds(scene.Start);
                VideoSurface.Play();
            }
        }

        // =====================================================================
        // Steps
        // =====================================================================

        private void ShowStep(Step step, bool animate = true)
        {
            _currentStep = step;

            var roots = new[] { StepWelcome, StepLicense, StepOptions, StepInstalling, StepFinish, StepError };
            foreach (var root in roots)
            {
                root.Visibility = Visibility.Collapsed;
            }

            var target = RootFor(step);
            target.Visibility = Visibility.Visible;

            if (animate && !_reducedMotion)
            {
                target.Opacity = 0;
                var transform = new TranslateTransform(0, 12);
                target.RenderTransform = transform;

                target.BeginAnimation(
                    UIElement.OpacityProperty,
                    new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(240))
                    {
                        EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
                    });

                transform.BeginAnimation(
                    TranslateTransform.YProperty,
                    new DoubleAnimation(12, 0, TimeSpan.FromMilliseconds(340))
                    {
                        EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
                    });
            }
            else
            {
                target.BeginAnimation(UIElement.OpacityProperty, null);
                target.Opacity = 1;
                target.RenderTransform = null;
            }

            UpdateStepper(step);
            SetPosterForStep(step);
        }

        private FrameworkElement RootFor(Step step)
        {
            switch (step)
            {
                case Step.Welcome:
                    return StepWelcome;
                case Step.License:
                    return StepLicense;
                case Step.Options:
                    return StepOptions;
                case Step.Installing:
                    return StepInstalling;
                case Step.Finish:
                    return StepFinish;
                default:
                    return StepError;
            }
        }

        private void UpdateStepper(Step step)
        {
            int index;
            string nameKey;

            switch (step)
            {
                case Step.Welcome:
                    index = 1;
                    nameKey = "StepWelcome";
                    break;
                case Step.License:
                    index = 2;
                    nameKey = "StepLicense";
                    break;
                case Step.Options:
                    index = 3;
                    nameKey = "StepOptions";
                    break;
                case Step.Installing:
                    index = 4;
                    nameKey = "StepInstalling";
                    break;
                case Step.Finish:
                    index = 5;
                    nameKey = "StepFinish";
                    break;
                default:
                    index = 4;
                    nameKey = "StepError";
                    break;
            }

            var dots = new[] { StepDot1, StepDot2, StepDot3, StepDot4, StepDot5 };
            var active = (Brush)FindResource("IceBrush");

            for (var i = 0; i < dots.Length; i++)
            {
                if (i < index - 1)
                {
                    dots[i].Background = DotDoneBrush;
                }
                else if (i == index - 1)
                {
                    dots[i].Background = active;
                }
                else
                {
                    dots[i].Background = DotIdleBrush;
                }
            }

            StepText.Text = string.Format(CultureInfo.CurrentCulture, L["StepOfFormat"], index, L[nameKey]);
        }

        // =====================================================================
        // Navigation
        // =====================================================================

        private async void WelcomeStart_Click(object sender, RoutedEventArgs e)
        {
            ShowStep(Step.License);
            await GoToSceneAsync(Step.License, true);
            LicenseContinueButton.Focus();
        }

        private void LicenseAgree_Changed(object sender, RoutedEventArgs e)
        {
            LicenseContinueButton.IsEnabled = LicenseAgreeToggle.IsChecked == true;
        }

        private async void LicenseContinue_Click(object sender, RoutedEventArgs e)
        {
            if (LicenseAgreeToggle.IsChecked != true)
            {
                return;
            }

            ShowStep(Step.Options);
            await GoToSceneAsync(Step.Options, true);
            InstallButton.Focus();
        }

        private async void LicenseBack_Click(object sender, RoutedEventArgs e)
        {
            ShowStep(Step.Welcome);
            await GoToSceneAsync(Step.Welcome, true);
        }

        private async void OptionsBack_Click(object sender, RoutedEventArgs e)
        {
            ShowStep(Step.License);
            await GoToSceneAsync(Step.License, true);
        }

        private async void Retry_Click(object sender, RoutedEventArgs e)
        {
            ShowStep(Step.Options);
            await GoToSceneAsync(Step.Options, true);
        }

        private void Done_Click(object sender, RoutedEventArgs e)
        {
            _allowClose = true;
            Close();
        }

        private void Exit_Click(object sender, RoutedEventArgs e)
        {
            _allowClose = true;
            Close();
        }

        // =====================================================================
        // Options
        // =====================================================================

        private void InstallPath_TextChanged(object sender, TextChangedEventArgs e)
        {
            ClearPathError();
            UpdateExistingInstallNote();
        }

        private void UpdateExistingInstallNote()
        {
            var exists = false;
            var other = false;

            try
            {
                var dir = (InstallPathBox.Text ?? string.Empty).Trim();
                exists = dir.Length > 0
                    && Directory.Exists(dir)
                    && File.Exists(Path.Combine(dir, "marguerite-gui.exe"));

                if (dir.Length > 0 && File.Exists(Path.Combine(DefaultInstallDir, "marguerite-gui.exe")))
                {
                    var normalized = Path.GetFullPath(dir).TrimEnd('\\');
                    var standard = DefaultInstallDir.TrimEnd('\\');
                    other = !string.Equals(normalized, standard, StringComparison.OrdinalIgnoreCase);
                }
            }
            catch
            {
            }

            ExistingInstallNote.Visibility = exists ? Visibility.Visible : Visibility.Collapsed;

            if (other)
            {
                OtherInstallNote.Text = string.Format(CultureInfo.CurrentCulture, L["OtherInstall"], DefaultInstallDir);
                OtherInstallNote.Visibility = Visibility.Visible;
            }
            else
            {
                OtherInstallNote.Visibility = Visibility.Collapsed;
            }
        }

        private void ShowPathError(string message)
        {
            PathErrorText.Text = message;
            PathErrorText.Visibility = Visibility.Visible;
        }

        private void ClearPathError()
        {
            PathErrorText.Visibility = Visibility.Collapsed;
            PathErrorText.Text = string.Empty;
        }

        private void Browse_Click(object sender, RoutedEventArgs e)
        {
            var current = (InstallPathBox.Text ?? string.Empty).Trim();
            var selected = FolderPicker.Pick(this, L["BrowseTitle"], current);
            if (!string.IsNullOrWhiteSpace(selected))
            {
                InstallPathBox.Text = selected;
            }
        }

        private bool ValidateInstallPath(string dir, out string errorKey)
        {
            errorKey = null;

            if (string.IsNullOrWhiteSpace(dir) || !Path.IsPathRooted(dir) || dir.IndexOf('"') >= 0)
            {
                errorKey = "PathInvalid";
                return false;
            }

            try
            {
                var full = Path.GetFullPath(dir);
                if (File.Exists(full))
                {
                    errorKey = "PathInvalid";
                    return false;
                }

                var root = Path.GetPathRoot(full);
                if (string.IsNullOrEmpty(root)
                    || string.Equals(full.TrimEnd('\\'), root.TrimEnd('\\'), StringComparison.OrdinalIgnoreCase))
                {
                    errorKey = "PathInvalid";
                    return false;
                }

                foreach (var invalid in Path.GetInvalidPathChars())
                {
                    if (full.IndexOf(invalid) >= 0)
                    {
                        errorKey = "PathInvalid";
                        return false;
                    }
                }

                Directory.CreateDirectory(full);
                var probe = Path.Combine(full, ".marguerite-setup-probe-" + Guid.NewGuid().ToString("N"));
                File.WriteAllText(probe, "probe");
                File.Delete(probe);
                return true;
            }
            catch
            {
                errorKey = "PathNotWritable";
                return false;
            }
        }

        private void UpdatePayloadStatus()
        {
            if (_payload == null)
            {
                PayloadDot.Fill = (Brush)FindResource("DangerBrush");
                PayloadNameText.Text = L["PayloadMissing"];
                PayloadActionButton.Content = L["ChooseFile"];
                return;
            }

            var pinned = string.Equals(_payload.Version, InstallerRunner.ExpectedVersion, StringComparison.OrdinalIgnoreCase);
            PayloadDot.Fill = (Brush)FindResource(pinned ? "IceBrush" : "WarnBrush");
            PayloadNameText.Text = _payload.FileName + " · " + L[pinned ? "PayloadPending" : "PayloadUnverified"];
            PayloadActionButton.Content = L["ChangePayload"];
        }

        private async void PayloadAction_Click(object sender, RoutedEventArgs e)
        {
            await PickPayloadAsync();
        }

        private async Task<bool> PromptForPayloadAsync()
        {
            var wants = await ShowModalAsync(
                L["PayloadMissingTitle"],
                L["PayloadMissingBody"],
                L["ChooseFile"],
                L["Back"]);

            if (!wants)
            {
                return false;
            }

            return await PickPayloadAsync();
        }

        private async Task<bool> PickPayloadAsync()
        {
            using (var dialog = new WinForms.OpenFileDialog())
            {
                dialog.Title = L["PayloadLabel"];
                dialog.Filter = "Marguerite installer|Marguerite_*_x64-setup.exe|Executable files|*.exe";
                dialog.CheckFileExists = true;

                if (dialog.ShowDialog(new Win32Window(this)) != WinForms.DialogResult.OK)
                {
                    return false;
                }

                var info = InstallerRunner.DescribePayload(dialog.FileName);
                if (info == null)
                {
                    await ShowModalAsync(L["PayloadInvalidTitle"], L["PayloadInvalidBody"], L["Ok"], null);
                    return false;
                }

                if (!string.Equals(info.Version, InstallerRunner.ExpectedVersion, StringComparison.OrdinalIgnoreCase)
                    && !File.Exists(info.Path + ".sha256"))
                {
                    await ShowModalAsync(
                        L["PayloadVersionTitle"],
                        string.Format(CultureInfo.CurrentCulture, L["PayloadVersionBody"], info.Version),
                        L["Ok"],
                        null);
                    return false;
                }

                _payload = info;
                UpdatePayloadStatus();
                return true;
            }
        }

        private static PayloadInfo ResolvePayload()
        {
            if (!string.IsNullOrWhiteSpace(RuntimeOptions.PayloadOverride))
            {
                var info = InstallerRunner.DescribePayload(RuntimeOptions.PayloadOverride);
                if (info != null)
                {
                    return info;
                }
            }

            return InstallerRunner.FindPayload(AppDomain.CurrentDomain.BaseDirectory);
        }

        // =====================================================================
        // Installation
        // =====================================================================

        private async void Install_Click(object sender, RoutedEventArgs e)
        {
            if (_isInstalling)
            {
                return;
            }

            var dir = (InstallPathBox.Text ?? string.Empty).Trim();
            string errorKey;
            if (!ValidateInstallPath(dir, out errorKey))
            {
                ShowPathError(L[errorKey]);
                return;
            }

            dir = Path.GetFullPath(dir);
            InstallPathBox.Text = dir;
            ClearPathError();

            if (!RuntimeOptions.Demo)
            {
                if (_payload == null)
                {
                    var chosen = await PromptForPayloadAsync();
                    if (!chosen)
                    {
                        return;
                    }
                }

                if (IsMargueriteRunning())
                {
                    var kill = await ShowModalAsync(
                        L["AppRunningTitle"],
                        L["AppRunningBody"],
                        L["KillAndContinue"],
                        L["Back"],
                        true);

                    if (!kill)
                    {
                        return;
                    }

                    await Task.Run(() => KillMarguerite());
                }
            }

            _percent = 0;
            _lastStage = InstallStage.Verifying;
            _installStart = DateTime.Now;
            _isInstalling = true;
            _installCts = new CancellationTokenSource();

            ShowStep(Step.Installing);
            await GoToSceneAsync(Step.Installing, true);

            UpdateProgress(0, InstallStage.Verifying);
            ElapsedText.Text = string.Format(CultureInfo.CurrentCulture, L["ElapsedFormat"], "00:00");
            CancelInstallButton.IsEnabled = true;
            _elapsedTimer.Start();

            var progress = new Progress<InstallProgress>(p => UpdateProgress(p.Percent, p.Stage));
            InstallResult result;

            try
            {
                if (RuntimeOptions.Demo)
                {
                    result = await RunDemoInstallAsync(dir, progress, _installCts.Token);
                }
                else
                {
                    _runner = new InstallerRunner();
                    result = await _runner.RunAsync(
                        _payload,
                        dir,
                        ShortcutsToggle.IsChecked == true,
                        progress,
                        _installCts.Token);
                }
            }
            catch (OperationCanceledException)
            {
                _isInstalling = false;
                _elapsedTimer.Stop();
                CleanupRunner();
                ShowStep(Step.Options);
                await GoToSceneAsync(Step.Options, true);
                return;
            }
            catch (Exception ex)
            {
                result = new InstallResult
                {
                    Success = false,
                    ErrorCode = "exception",
                    Detail = ex.Message,
                    LogPath = _runner != null ? _runner.LogPath : null,
                    InstallDir = dir
                };
            }

            _isInstalling = false;
            _elapsedTimer.Stop();
            CleanupRunner();

            if (result.Success)
            {
                _installedDir = dir;
                ShowFinish();
                await GoToSceneAsync(Step.Finish, true);
            }
            else
            {
                ShowError(result);
                await GoToSceneAsync(Step.Error, true);
            }
        }

        private async Task<InstallResult> RunDemoInstallAsync(
            string installDir,
            IProgress<InstallProgress> progress,
            CancellationToken cancellationToken)
        {
            for (var i = 0; i <= 100; i++)
            {
                cancellationToken.ThrowIfCancellationRequested();

                InstallStage stage;
                if (i < 4)
                {
                    stage = InstallStage.Verifying;
                }
                else if (i < 14)
                {
                    stage = InstallStage.Preparing;
                }
                else if (i < 46)
                {
                    stage = InstallStage.Extracting;
                }
                else if (i < 82)
                {
                    stage = InstallStage.Writing;
                }
                else if (i < 96)
                {
                    stage = InstallStage.Registering;
                }
                else
                {
                    stage = InstallStage.Finalizing;
                }

                progress.Report(new InstallProgress { Percent = i, Stage = stage });
                await Task.Delay(120, cancellationToken);
            }

            return new InstallResult
            {
                Success = true,
                InstallDir = installDir,
                LogPath = "(demo)"
            };
        }

        private void UpdateProgress(double percent, InstallStage stage)
        {
            if (percent > _percent || percent >= 100)
            {
                _percent = Math.Max(0, Math.Min(100, percent));
            }

            _lastStage = stage;

            PercentText.Text = ((int)Math.Round(_percent)).ToString(CultureInfo.InvariantCulture);

            var animation = new DoubleAnimation(_percent, TimeSpan.FromMilliseconds(300))
            {
                EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
            };
            ProgressBar.BeginAnimation(System.Windows.Controls.Primitives.RangeBase.ValueProperty, animation);

            var key = StageKey(stage);
            if (stage == InstallStage.Preparing
                && !RuntimeOptions.Demo
                && _percent < 6
                && _installStart != default(DateTime)
                && (DateTime.Now - _installStart).TotalSeconds > 22)
            {
                key = "StageWebView";
            }

            StageText.Text = L[key];
        }

        private static string StageKey(InstallStage stage)
        {
            switch (stage)
            {
                case InstallStage.Verifying:
                    return "StageVerifying";
                case InstallStage.Extracting:
                    return "StageExtracting";
                case InstallStage.Writing:
                    return "StageWriting";
                case InstallStage.Registering:
                    return "StageRegistering";
                case InstallStage.Finalizing:
                    return "StageFinalizing";
                default:
                    return "StagePreparing";
            }
        }

        private void ElapsedTimer_Tick(object sender, EventArgs e)
        {
            if (!_isInstalling || _installStart == default(DateTime))
            {
                return;
            }

            var elapsed = DateTime.Now - _installStart;
            ElapsedText.Text = string.Format(
                CultureInfo.CurrentCulture,
                L["ElapsedFormat"],
                elapsed.ToString(@"mm\:ss"));
        }

        private static bool IsMargueriteRunning()
        {
            try
            {
                return Process.GetProcessesByName("marguerite-gui").Length > 0;
            }
            catch
            {
                return false;
            }
        }

        private static void KillMarguerite()
        {
            var processes = Process.GetProcessesByName("marguerite-gui");
            foreach (var process in processes)
            {
                try
                {
                    process.CloseMainWindow();
                }
                catch
                {
                }
            }

            Thread.Sleep(1200);

            processes = Process.GetProcessesByName("marguerite-gui");
            foreach (var process in processes)
            {
                try
                {
                    if (!process.HasExited)
                    {
                        process.Kill();
                    }
                }
                catch
                {
                }
                finally
                {
                    process.Dispose();
                }
            }
        }

        private async void CancelInstall_Click(object sender, RoutedEventArgs e)
        {
            if (!_isInstalling)
            {
                return;
            }

            var cancel = await ShowModalAsync(
                L["CancelConfirmTitle"],
                L["CancelConfirmBody"],
                L["QuitAnyway"],
                L["KeepInstalling"],
                true);

            if (!cancel)
            {
                return;
            }

            StageText.Text = L["StageCancelling"];
            CancelInstallButton.IsEnabled = false;

            try
            {
                _installCts.Cancel();
            }
            catch
            {
            }
        }

        private void CleanupRunner()
        {
            if (_runner == null)
            {
                return;
            }

            try
            {
                _runner.Dispose();
            }
            catch
            {
            }

            _runner = null;
        }

        // =====================================================================
        // Finish / error
        // =====================================================================

        private void ShowFinish()
        {
            _installedDir = _installedDir ?? DefaultInstallDir;
            FinishBodyText.Text = string.Format(CultureInfo.CurrentCulture, L["FinishBody"], _installedDir);

            var main = Path.Combine(_installedDir, "marguerite-gui.exe");
            LaunchButton.IsEnabled = File.Exists(main);

            ShowStep(Step.Finish);

            if (!_reducedMotion)
            {
                FinishMarkScale.ScaleX = 0.7;
                FinishMarkScale.ScaleY = 0.7;
                var ease = new BackEase { Amplitude = 0.3, EasingMode = EasingMode.EaseOut };
                var duration = TimeSpan.FromMilliseconds(520);

                FinishMarkScale.BeginAnimation(
                    ScaleTransform.ScaleXProperty,
                    new DoubleAnimation(0.7, 1, duration) { EasingFunction = ease });
                FinishMarkScale.BeginAnimation(
                    ScaleTransform.ScaleYProperty,
                    new DoubleAnimation(0.7, 1, duration) { EasingFunction = ease });
            }
            else
            {
                FinishMarkScale.ScaleX = 1;
                FinishMarkScale.ScaleY = 1;
            }
        }

        private void ShowError(InstallResult result)
        {
            _lastError = result;
            ErrorBodyText.Text = DescribeError(result);
            ErrorLogText.Text = string.IsNullOrWhiteSpace(result.LogPath)
                ? string.Empty
                : string.Format(CultureInfo.CurrentCulture, L["LogFormat"], result.LogPath);
            ShowStep(Step.Error);
        }

        private string DescribeError(InstallResult result)
        {
            switch (result.ErrorCode)
            {
                case "payload_hash":
                    return string.Format(CultureInfo.CurrentCulture, L["ErrPayloadHash"], result.Detail, result.Detail2);
                case "exit_code":
                    return string.Format(CultureInfo.CurrentCulture, L["ErrExitCode"], result.ExitCode);
                case "main_missing":
                    return L["ErrMainBinaryMissing"];
                case "payload_missing":
                    return L["ErrPayloadMissing"];
                case "spawn":
                    return string.Format(CultureInfo.CurrentCulture, L["ErrSpawn"], result.Detail ?? string.Empty);
                case "payload_version":
                    return string.Format(CultureInfo.CurrentCulture, L["ErrPayloadVersion"], result.Detail ?? string.Empty);
                default:
                    return string.Format(
                        CultureInfo.CurrentCulture,
                        L["ErrUnknown"],
                        result.Detail ?? result.ExitCode.ToString(CultureInfo.InvariantCulture));
            }
        }

        private void Launch_Click(object sender, RoutedEventArgs e)
        {
            var dir = _installedDir ?? DefaultInstallDir;
            var exe = Path.Combine(dir, "marguerite-gui.exe");

            if (!File.Exists(exe))
            {
                ShowError(new InstallResult { ErrorCode = "main_missing", Detail = exe });
                return;
            }

            try
            {
                Process.Start(new ProcessStartInfo
                {
                    FileName = exe,
                    WorkingDirectory = dir,
                    UseShellExecute = true
                });
            }
            catch (Exception ex)
            {
                ShowError(new InstallResult { ErrorCode = "exception", Detail = ex.Message });
                return;
            }

            _allowClose = true;
            Close();
        }

        private void OpenFolder_Click(object sender, RoutedEventArgs e)
        {
            var dir = _installedDir ?? DefaultInstallDir;
            try
            {
                if (Directory.Exists(dir))
                {
                    Process.Start(new ProcessStartInfo
                    {
                        FileName = "explorer.exe",
                        Arguments = "\"" + dir + "\"",
                        UseShellExecute = true
                    });
                }
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

            if (_audioOn)
            {
                VideoSurface.Volume = 0.35;
                if (_videoReady && !_videoFailed && !_reducedMotion)
                {
                    VideoSurface.Play();
                }
            }

            UpdateSoundUi();
        }

        private void UpdateSoundUi()
        {
            SoundGlyph.Text = _audioOn ? "\uE767" : "\uE74F";
            SoundButton.ToolTip = _audioOn ? L["SoundMuteTip"] : L["SoundOnTip"];
            MinimizeButton.ToolTip = L["MinimizeTip"];
            CloseButton.ToolTip = L["CloseTip"];
        }

        private void Language_Click(object sender, RoutedEventArgs e)
        {
            _loc.Toggle();
            LanguageButtonText.Text = _loc.LanguageButton;
            UpdateStepper(_currentStep);
            UpdatePayloadStatus();
            UpdateExistingInstallNote();
            UpdateSoundUi();

            if (!string.IsNullOrEmpty(_installedDir) && StepFinish.Visibility == Visibility.Visible)
            {
                FinishBodyText.Text = string.Format(CultureInfo.CurrentCulture, L["FinishBody"], _installedDir);
            }

            if (_lastError != null && StepError.Visibility == Visibility.Visible)
            {
                ErrorBodyText.Text = DescribeError(_lastError);
                ErrorLogText.Text = string.IsNullOrWhiteSpace(_lastError.LogPath)
                    ? string.Empty
                    : string.Format(CultureInfo.CurrentCulture, L["LogFormat"], _lastError.LogPath);
            }

            if (StepInstalling.Visibility == Visibility.Visible)
            {
                StageText.Text = L[StageKey(_lastStage)];
            }
        }

        private void MainWindow_PreviewKeyDown(object sender, KeyEventArgs e)
        {
            if (e.Key != Key.Escape)
            {
                return;
            }

            if (ModalLayer.Visibility == Visibility.Visible)
            {
                var target = ModalSecondary.Visibility == Visibility.Visible ? ModalSecondary : ModalPrimary;
                target.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                e.Handled = true;
                return;
            }

            if (_isInstalling)
            {
                Close();
            }
            else
            {
                _allowClose = true;
                Close();
            }
        }

        private void MainWindow_Closing(object sender, CancelEventArgs e)
        {
            if (!_isInstalling || _allowClose)
            {
                return;
            }

            e.Cancel = true;

            if (!_closePromptOpen)
            {
                _closePromptOpen = true;
                _ = PromptCloseDuringInstallAsync();
            }
        }

        private async Task PromptCloseDuringInstallAsync()
        {
            var quit = await ShowModalAsync(
                L["CloseInstallTitle"],
                L["CloseInstallBody"],
                L["QuitAnyway"],
                L["KeepInstalling"],
                true);

            _closePromptOpen = false;

            if (!quit)
            {
                return;
            }

            try
            {
                _installCts.Cancel();
            }
            catch
            {
            }

            _allowClose = true;
            Close();
        }

        // =====================================================================
        // Modal
        // =====================================================================

        private Task<bool> ShowModalAsync(
            string title,
            string body,
            string primary,
            string secondary,
            bool dangerPrimary = false)
        {
            ModalTitle.Text = title;
            ModalBody.Text = body;
            ModalPrimary.Content = primary;
            ModalSecondary.Content = secondary;
            ModalSecondary.Visibility = string.IsNullOrWhiteSpace(secondary) ? Visibility.Collapsed : Visibility.Visible;

            ModalPrimary.Background = dangerPrimary
                ? (Brush)FindResource("DangerBrush")
                : (Brush)FindResource("IvoryBrush");
            ModalPrimary.Foreground = dangerPrimary
                ? Brushes.White
                : (Brush)FindResource("InkBrush");

            ModalLayer.Visibility = Visibility.Visible;
            ModalLayer.Opacity = 0;
            ModalLayer.BeginAnimation(
                UIElement.OpacityProperty,
                new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(160))
                {
                    EasingFunction = new CubicEase { EasingMode = EasingMode.EaseOut }
                });

            ModalPrimary.Focus();

            var completion = new TaskCompletionSource<bool>();
            RoutedEventHandler onPrimary = null;
            RoutedEventHandler onSecondary = null;
            Action cleanup = null;

            cleanup = () =>
            {
                ModalPrimary.Click -= onPrimary;
                ModalSecondary.Click -= onSecondary;
                ModalLayer.Visibility = Visibility.Collapsed;
            };

            onPrimary = (sender, args) =>
            {
                cleanup();
                completion.TrySetResult(true);
            };

            onSecondary = (sender, args) =>
            {
                cleanup();
                completion.TrySetResult(false);
            };

            ModalPrimary.Click += onPrimary;
            ModalSecondary.Click += onSecondary;

            return completion.Task;
        }

        // =====================================================================
        // Native interop
        // =====================================================================

        [DllImport("dwmapi.dll")]
        private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attribute, ref int value, int size);

        private sealed class Win32Window : WinForms.IWin32Window
        {
            private readonly IntPtr _handle;

            public Win32Window(Window window)
            {
                _handle = new WindowInteropHelper(window).Handle;
            }

            public IntPtr Handle
            {
                get { return _handle; }
            }
        }
    }
}
