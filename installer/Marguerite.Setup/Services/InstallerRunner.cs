using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace MargueriteSetup
{
    public enum InstallStage
    {
        Verifying,
        Preparing,
        Extracting,
        Writing,
        Registering,
        Finalizing
    }

    public sealed class InstallProgress
    {
        public double Percent { get; set; }

        public InstallStage Stage { get; set; }
    }

    public sealed class PayloadInfo
    {
        public string Path { get; set; }

        public string FileName { get; set; }

        public string Version { get; set; }
    }

    public sealed class InstallResult
    {
        public bool Success { get; set; }

        public string ErrorCode { get; set; }

        public string Detail { get; set; }

        public string Detail2 { get; set; }

        public int ExitCode { get; set; }

        public string LogPath { get; set; }

        public string InstallDir { get; set; }
    }

    /// <summary>
    /// Drives the existing Tauri NSIS installer in silent mode. The visual shell
    /// owns the experience; the NSIS payload owns the actual machine changes
    /// (files, shortcuts, uninstall entry, WebView2 runtime).
    /// </summary>
    public sealed class InstallerRunner : IDisposable
    {
        public const string ExpectedVersion = "0.2.17";

        public const string ExpectedSha256 = "a55bd21b6dd37aa8884d941135f2e607f714e8771b17da6810f427364261c2d0";

        // ESTIMATEDSIZE in the generated installer.nsi is 372497 KB.
        public const long EstimatedInstalledBytes = 372497L * 1024L;

        private static readonly Regex PayloadName = new Regex(
            @"^Marguerite_(\d+\.\d+\.\d+)_x64-setup\.exe$",
            RegexOptions.IgnoreCase | RegexOptions.CultureInvariant | RegexOptions.Compiled);

        private readonly StreamWriter _log;

        private JobObject _job;

        private string _installDir;

        public InstallerRunner()
        {
            var dir = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "MargueriteSetup");
            Directory.CreateDirectory(dir);
            LogPath = System.IO.Path.Combine(dir, "setup-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".log");
            _log = new StreamWriter(LogPath, false, new UTF8Encoding(false)) { AutoFlush = true };
        }

        public string LogPath { get; private set; }

        public static PayloadInfo DescribePayload(string path)
        {
            if (string.IsNullOrWhiteSpace(path) || !File.Exists(path))
            {
                return null;
            }

            var name = System.IO.Path.GetFileName(path);
            var match = PayloadName.Match(name ?? string.Empty);
            if (!match.Success)
            {
                return null;
            }

            return new PayloadInfo
            {
                Path = System.IO.Path.GetFullPath(path),
                FileName = name,
                Version = match.Groups[1].Value
            };
        }

        /// <summary>
        /// Finds the co-located payload. The setup shell ships as two files:
        /// MargueriteSetup.exe plus the untouched Tauri NSIS installer.
        /// </summary>
        public static PayloadInfo FindPayload(string directory)
        {
            if (!string.IsNullOrWhiteSpace(RuntimeOptions.PayloadOverride))
            {
                return DescribePayload(RuntimeOptions.PayloadOverride);
            }

            if (string.IsNullOrWhiteSpace(directory) || !Directory.Exists(directory))
            {
                return null;
            }

            var found = Directory
                .EnumerateFiles(directory, "Marguerite_*_x64-setup.exe", SearchOption.TopDirectoryOnly)
                .Select(DescribePayload)
                .Where(p => p != null)
                .OrderByDescending(p => VersionOf(p.Version))
                .FirstOrDefault();

            return found;
        }

        public static string BuildArguments(string installDir, bool shortcuts)
        {
            var dir = (installDir ?? string.Empty).Trim();
            if (dir.Length > 3 && (dir.EndsWith("\\", StringComparison.Ordinal) || dir.EndsWith("/", StringComparison.Ordinal)))
            {
                dir = dir.Substring(0, dir.Length - 1);
            }

            // NSIS: /D must be the last switch and must not be quoted; the rest of
            // the command line is taken literally as the install directory.
            return "/S" + (shortcuts ? string.Empty : " /NS") + " /D=" + dir;
        }

        public static long GetDirectorySize(string dir)
        {
            long total = 0;
            try
            {
                var root = new DirectoryInfo(dir);
                if (!root.Exists)
                {
                    return 0;
                }

                foreach (var file in root.EnumerateFiles("*", SearchOption.AllDirectories))
                {
                    try
                    {
                        total += file.Length;
                    }
                    catch
                    {
                        // A file can be replaced while the installer is writing it.
                    }
                }
            }
            catch
            {
                // A missing/permission-denied subtree simply contributes nothing.
            }

            return total;
        }

        public async Task<InstallResult> RunAsync(
            PayloadInfo payload,
            string installDir,
            bool shortcuts,
            IProgress<InstallProgress> progress,
            CancellationToken cancellationToken)
        {
            _installDir = installDir;

            if (payload == null || !File.Exists(payload.Path))
            {
                return Fail("payload_missing", payload == null ? null : payload.Path, null, 0);
            }

            // The shell is pinned to 0.2.17. A payload with another version is
            // only allowed when it carries its own sidecar checksum, otherwise
            // the UI could claim 0.2.17 while installing something else.
            if (!string.Equals(payload.Version, ExpectedVersion, StringComparison.OrdinalIgnoreCase)
                && !File.Exists(payload.Path + ".sha256"))
            {
                return Fail("payload_version", payload.Version, null, 0);
            }

            var arguments = BuildArguments(installDir, shortcuts);
            Log("payload=" + payload.Path);
            Log("version=" + payload.Version);
            Log("arguments=" + arguments);

            Report(progress, 0, InstallStage.Verifying);

            HashCheck check;
            try
            {
                check = await VerifyPayloadAsync(payload, progress, cancellationToken);
            }
            catch (OperationCanceledException)
            {
                throw;
            }

            if (!check.Ok)
            {
                return Fail("payload_hash", check.Expected, check.Actual, 0);
            }

            Log(check.Verified ? "sha256 verified" : "sha256 skipped (payload version has no pinned hash)");

            try
            {
                Directory.CreateDirectory(installDir);
            }
            catch (Exception ex)
            {
                return Fail("path", ex.Message, null, 0);
            }

            // The watcher must exist before the payload starts, otherwise the
            // first files the installer writes are silently missed.
            WrittenBytesTracker tracker = null;
            try
            {
                tracker = new WrittenBytesTracker(installDir);
            }
            catch (Exception ex)
            {
                Log("watcher unavailable: " + ex.Message);
            }

            var startInfo = new ProcessStartInfo
            {
                FileName = payload.Path,
                Arguments = arguments,
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = System.IO.Path.GetDirectoryName(payload.Path) ?? AppDomain.CurrentDomain.BaseDirectory
            };

            Process process;
            try
            {
                process = new Process { StartInfo = startInfo, EnableRaisingEvents = true };
                if (!process.Start())
                {
                    process.Dispose();
                    if (tracker != null)
                    {
                        tracker.Dispose();
                    }

                    return Fail("spawn", "Process.Start returned false", null, 0);
                }
            }
            catch (Exception ex)
            {
                Log("spawn failed: " + ex.Message);
                if (tracker != null)
                {
                    tracker.Dispose();
                }

                return Fail("spawn", ex.Message, null, 0);
            }

            using (tracker)
            using (process)
            {
                _job = new JobObject();
                _job.Assign(process);
                Log("pid=" + process.Id);

                var waitTask = Task.Run(() => process.WaitForExit());
                var stopwatch = Stopwatch.StartNew();
                var lastStage = InstallStage.Verifying;

                try
                {
                    while (!waitTask.IsCompleted)
                    {
                        cancellationToken.ThrowIfCancellationRequested();

                        // A FileSystemWatcher only counts files the payload
                        // writes now. It is accurate for upgrades too, where a
                        // full directory scan would immediately report 100%
                        // because of pre-existing user data.
                        var bytes = tracker != null ? tracker.TotalBytes : GetDirectorySize(installDir);
                        var ratio = EstimatedInstalledBytes <= 0
                            ? 0
                            : Math.Min(1.0, bytes / (double)EstimatedInstalledBytes);
                        var stage = StageFor(ratio, stopwatch.Elapsed, bytes);
                        var percent = Math.Min(99.0, 5.0 + 94.0 * ratio);

                        if (stage != lastStage)
                        {
                            Log("stage=" + stage + " bytes=" + bytes + " pct=" + percent.ToString("0.0"));
                            lastStage = stage;
                        }

                        Report(progress, percent, stage);

                        try
                        {
                            await Task.Delay(450, cancellationToken);
                        }
                        catch (OperationCanceledException)
                        {
                            TerminateChild();
                            throw;
                        }
                    }

                    await waitTask;
                }
                catch (OperationCanceledException)
                {
                    TerminateChild();
                    throw;
                }
                catch (Exception ex)
                {
                    TerminateChild();
                    return Fail("exception", ex.Message, null, process.HasExited ? process.ExitCode : 0);
                }

                stopwatch.Stop();
                var exitCode = process.ExitCode;
                Log("exit=" + exitCode + " elapsed=" + stopwatch.Elapsed);

                if (exitCode != 0)
                {
                    return Fail("exit_code", null, null, exitCode);
                }

                var main = System.IO.Path.Combine(installDir, "marguerite-gui.exe");
                if (!File.Exists(main))
                {
                    return Fail("main_missing", main, null, exitCode);
                }

                Report(progress, 100, InstallStage.Finalizing);
                Log("success main=" + main);

                return new InstallResult
                {
                    Success = true,
                    ExitCode = 0,
                    InstallDir = installDir,
                    LogPath = LogPath
                };
            }
        }

        public void Dispose()
        {
            if (_job != null)
            {
                _job.Dispose();
                _job = null;
            }

            if (_log != null)
            {
                try
                {
                    _log.Dispose();
                }
                catch
                {
                }
            }
        }

        private static Version VersionOf(string text)
        {
            Version parsed;
            return Version.TryParse(text, out parsed) ? parsed : new Version(0, 0);
        }

        private static InstallStage StageFor(double ratio, TimeSpan elapsed, long bytes)
        {
            if (bytes < 1 && elapsed.TotalSeconds > 22)
            {
                // NSIS may be installing the offline WebView2 runtime before it
                // writes anything into the destination directory.
                return InstallStage.Preparing;
            }

            if (ratio < 0.02)
            {
                return InstallStage.Preparing;
            }

            if (ratio < 0.35)
            {
                return InstallStage.Extracting;
            }

            if (ratio < 0.85)
            {
                return InstallStage.Writing;
            }

            if (ratio < 0.995)
            {
                return InstallStage.Registering;
            }

            return InstallStage.Finalizing;
        }

        private static void Report(IProgress<InstallProgress> progress, double percent, InstallStage stage)
        {
            if (progress != null)
            {
                progress.Report(new InstallProgress
                {
                    Percent = Math.Max(0, Math.Min(100, percent)),
                    Stage = stage
                });
            }
        }

        private async Task<HashCheck> VerifyPayloadAsync(
            PayloadInfo payload,
            IProgress<InstallProgress> progress,
            CancellationToken cancellationToken)
        {
            string expected = null;
            var sidecar = payload.Path + ".sha256";
            if (File.Exists(sidecar))
            {
                var line = File.ReadAllText(sidecar).Trim();
                var token = line.Split(new[] { ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries).FirstOrDefault();
                if (!string.IsNullOrWhiteSpace(token))
                {
                    expected = token;
                }
            }
            else if (string.Equals(payload.Version, ExpectedVersion, StringComparison.OrdinalIgnoreCase))
            {
                expected = ExpectedSha256;
            }

            if (expected == null)
            {
                return new HashCheck { Ok = true, Verified = false };
            }

            Log("hashing with expected=" + expected);
            var total = new FileInfo(payload.Path).Length;
            var actual = await ComputeSha256Async(
                payload.Path,
                done =>
                {
                    var percent = total <= 0 ? 0 : 4.0 * done / total;
                    Report(progress, percent, InstallStage.Verifying);
                },
                cancellationToken);

            if (!string.Equals(expected, actual, StringComparison.OrdinalIgnoreCase))
            {
                Log("hash mismatch actual=" + actual);
                return new HashCheck { Ok = false, Expected = expected, Actual = actual };
            }

            Report(progress, 4, InstallStage.Verifying);
            return new HashCheck { Ok = true, Verified = true };
        }

        private static Task<string> ComputeSha256Async(
            string path,
            Action<long> onProgress,
            CancellationToken cancellationToken)
        {
            return Task.Run(
                () =>
                {
                    using (var sha = SHA256.Create())
                    using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 20, FileOptions.SequentialScan))
                    {
                        var buffer = new byte[1 << 20];
                        long done = 0;
                        int read;
                        while ((read = stream.Read(buffer, 0, buffer.Length)) > 0)
                        {
                            cancellationToken.ThrowIfCancellationRequested();
                            sha.TransformBlock(buffer, 0, read, null, 0);
                            done += read;
                            if (onProgress != null)
                            {
                                onProgress(done);
                            }
                        }

                        sha.TransformFinalBlock(new byte[0], 0, 0);
                        return ToHex(sha.Hash);
                    }
                },
                cancellationToken);
        }

        private static string ToHex(byte[] bytes)
        {
            if (bytes == null)
            {
                return string.Empty;
            }

            var builder = new StringBuilder(bytes.Length * 2);
            foreach (var b in bytes)
            {
                builder.Append(b.ToString("x2"));
            }

            return builder.ToString();
        }

        private InstallResult Fail(string code, string detail, string detail2, int exitCode)
        {
            Log("FAIL code=" + code + " detail=" + (detail ?? "-") + " detail2=" + (detail2 ?? "-") + " exit=" + exitCode);
            return new InstallResult
            {
                Success = false,
                ErrorCode = code,
                Detail = detail,
                Detail2 = detail2,
                ExitCode = exitCode,
                LogPath = LogPath,
                InstallDir = _installDir
            };
        }

        private void TerminateChild()
        {
            if (_job != null)
            {
                _job.Terminate();
            }
        }

        private void Log(string line)
        {
            if (_log == null)
            {
                return;
            }

            try
            {
                _log.WriteLine(DateTime.Now.ToString("HH:mm:ss.fff") + "  " + line);
            }
            catch
            {
            }
        }

        private sealed class HashCheck
        {
            public bool Ok { get; set; }

            public bool Verified { get; set; }

            public string Expected { get; set; }

            public string Actual { get; set; }
        }

        /// <summary>
        /// Counts only the bytes the payload writes from this moment on. Unlike
        /// a total directory size, this stays honest when the destination
        /// already holds a large user profile (sessions, settings, models).
        /// </summary>
        private sealed class WrittenBytesTracker : IDisposable
        {
            private readonly ConcurrentDictionary<string, long> _sizes =
                new ConcurrentDictionary<string, long>(StringComparer.OrdinalIgnoreCase);

            private readonly FileSystemWatcher _watcher;

            public WrittenBytesTracker(string directory)
            {
                Directory.CreateDirectory(directory);

                _watcher = new FileSystemWatcher(directory)
                {
                    IncludeSubdirectories = true,
                    NotifyFilter = NotifyFilters.FileName
                        | NotifyFilters.DirectoryName
                        | NotifyFilters.LastWrite
                        | NotifyFilters.Size,
                    InternalBufferSize = 64 * 1024,
                    EnableRaisingEvents = true
                };

                _watcher.Created += OnChanged;
                _watcher.Changed += OnChanged;
                _watcher.Deleted += OnDeleted;
                _watcher.Renamed += OnRenamed;
            }

            public long TotalBytes
            {
                get
                {
                    long total = 0;
                    foreach (var pair in _sizes)
                    {
                        total += pair.Value;
                    }

                    return total;
                }
            }

            public void Dispose()
            {
                try
                {
                    _watcher.EnableRaisingEvents = false;
                    _watcher.Dispose();
                }
                catch
                {
                }
            }

            private void OnChanged(object sender, FileSystemEventArgs e)
            {
                Update(e.FullPath);
            }

            private void OnRenamed(object sender, RenamedEventArgs e)
            {
                long ignored;
                _sizes.TryRemove(e.OldFullPath, out ignored);
                Update(e.FullPath);
            }

            private void OnDeleted(object sender, FileSystemEventArgs e)
            {
                long ignored;
                _sizes.TryRemove(e.FullPath, out ignored);
            }

            private void Update(string path)
            {
                try
                {
                    var file = new FileInfo(path);
                    if (file.Exists)
                    {
                        _sizes[path] = file.Length;
                    }
                }
                catch
                {
                    // The file may still be exclusively locked; the next Changed
                    // event will update it.
                }
            }
        }

        /// <summary>
        /// Kills the NSIS process and every child (including the WebView2
        /// installer) if this shell exits or the user cancels.
        /// </summary>
        private sealed class JobObject : IDisposable
        {
            private const uint JobObjectLimitKillOnJobClose = 0x00002000;
            private const int ExtendedLimitInformationClass = 9;

            private IntPtr _handle;

            public JobObject()
            {
                _handle = CreateJobObject(IntPtr.Zero, null);
                if (_handle == IntPtr.Zero)
                {
                    return;
                }

                var info = new JobObjectExtendedLimitInformation();
                info.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;

                var length = Marshal.SizeOf(typeof(JobObjectExtendedLimitInformation));
                var pointer = Marshal.AllocHGlobal(length);
                try
                {
                    Marshal.StructureToPtr(info, pointer, false);
                    SetInformationJobObject(_handle, ExtendedLimitInformationClass, pointer, (uint)length);
                }
                finally
                {
                    Marshal.FreeHGlobal(pointer);
                }
            }

            public void Assign(Process process)
            {
                if (_handle != IntPtr.Zero && process != null)
                {
                    try
                    {
                        AssignProcessToJobObject(_handle, process.Handle);
                    }
                    catch
                    {
                    }
                }
            }

            public void Terminate()
            {
                if (_handle != IntPtr.Zero)
                {
                    try
                    {
                        TerminateJobObject(_handle, 1);
                    }
                    catch
                    {
                    }
                }
            }

            public void Dispose()
            {
                if (_handle != IntPtr.Zero)
                {
                    try
                    {
                        CloseHandle(_handle);
                    }
                    catch
                    {
                    }

                    _handle = IntPtr.Zero;
                }
            }

            [StructLayout(LayoutKind.Sequential)]
            private struct JobObjectBasicLimitInformation
            {
                public long PerProcessUserTimeLimit;
                public long PerJobUserTimeLimit;
                public uint LimitFlags;
                public UIntPtr MinimumWorkingSetSize;
                public UIntPtr MaximumWorkingSetSize;
                public uint ActiveProcessLimit;
                public UIntPtr Affinity;
                public uint PriorityClass;
                public uint SchedulingClass;
            }

            [StructLayout(LayoutKind.Sequential)]
            private struct IoCounters
            {
                public ulong ReadOperationCount;
                public ulong WriteOperationCount;
                public ulong OtherOperationCount;
                public ulong ReadTransferCount;
                public ulong WriteTransferCount;
                public ulong OtherTransferCount;
            }

            [StructLayout(LayoutKind.Sequential)]
            private struct JobObjectExtendedLimitInformation
            {
                public JobObjectBasicLimitInformation BasicLimitInformation;
                public IoCounters IoInfo;
                public UIntPtr ProcessMemoryLimit;
                public UIntPtr JobMemoryLimit;
                public UIntPtr PeakProcessMemoryUsed;
                public UIntPtr PeakJobMemoryUsed;
            }

            [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
            private static extern IntPtr CreateJobObject(IntPtr securityAttributes, string name);

            [DllImport("kernel32.dll")]
            private static extern bool SetInformationJobObject(
                IntPtr job,
                int infoClass,
                IntPtr info,
                uint infoLength);

            [DllImport("kernel32.dll")]
            private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

            [DllImport("kernel32.dll")]
            private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

            [DllImport("kernel32.dll")]
            private static extern bool CloseHandle(IntPtr handle);
        }
    }
}
