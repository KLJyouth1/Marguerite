using System;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Cache;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace MargueriteDownloader
{
    public sealed class ReleaseAsset
    {
        public string Name { get; set; }

        public string Url { get; set; }

        public long Size { get; set; }

        public string Sha256 { get; set; }

        public string Tag { get; set; }
    }

    public sealed class DownloadProgressInfo
    {
        public long Received { get; set; }

        public long Total { get; set; }

        public double BytesPerSecond { get; set; }
    }

    /// <summary>
    /// Network side of the downloader. Everything is cancellable, and every
    /// call is retried so a flaky connection does not force the user to
    /// restart the film from the beginning.
    /// </summary>
    public sealed class DownloadEngine
    {
        private const int MaxAttempts = 3;

        static DownloadEngine()
        {
            // .NET 4.8 defaults to TLS 1.0 on older Windows builds; GitHub only
            // serves TLS 1.2+.
            ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
        }

        public sealed class ReleaseInfo
        {
            public string Tag { get; set; }

            public ReleaseAsset Installer { get; set; }

            public ReleaseAsset SetupShell { get; set; }
        }

        /// <summary>
        /// Reads the latest release once and picks the two assets this app needs:
        /// the NSIS payload and the cinematic setup shell.
        /// </summary>
        public async Task<ReleaseInfo> ResolveLatestAsync(string apiUrl, CancellationToken cancellationToken)
        {
            var json = await GetStringAsync(apiUrl, cancellationToken);
            var release = Json.AsObject(Json.Parse(json));

            if (release == null)
            {
                throw new InvalidOperationException("GitHub returned an empty release payload.");
            }

            var tag = Json.GetString(release, "tag_name") ?? string.Empty;
            var assets = Json.AsArray(release.ContainsKey("assets") ? release["assets"] : null);
            if (assets == null || assets.Count == 0)
            {
                throw new InvalidOperationException("The latest release has no assets yet.");
            }

            var info = new ReleaseInfo { Tag = tag };

            foreach (var raw in assets)
            {
                var asset = Json.AsObject(raw);
                if (asset == null)
                {
                    continue;
                }

                var name = Json.GetString(asset, "name");
                var url = Json.GetString(asset, "browser_download_url");
                if (string.IsNullOrWhiteSpace(name) || string.IsNullOrWhiteSpace(url))
                {
                    continue;
                }

                var entry = new ReleaseAsset
                {
                    Name = name,
                    Url = url,
                    Size = Json.GetLong(asset, "size"),
                    Sha256 = Json.GetString(asset, "digest"),
                    Tag = tag
                };

                if (Regex.IsMatch(name, @"^Marguerite_.+_x64-setup\.exe$", RegexOptions.IgnoreCase))
                {
                    info.Installer = entry;
                }
                else if (string.Equals(name, "MargueriteSetup.exe", StringComparison.OrdinalIgnoreCase))
                {
                    info.SetupShell = entry;
                }
            }

            if (info.Installer == null)
            {
                throw new InvalidOperationException("The latest release has no x64 installer asset.");
            }

            return info;
        }

        public async Task DownloadAsync(
            string url,
            string destination,
            IProgress<DownloadProgressInfo> progress,
            CancellationToken cancellationToken)
        {
            Exception lastError = null;

            for (var attempt = 1; attempt <= MaxAttempts; attempt++)
            {
                cancellationToken.ThrowIfCancellationRequested();

                try
                {
                    await DownloadOnceAsync(url, destination, progress, cancellationToken);
                    return;
                }
                catch (OperationCanceledException)
                {
                    throw;
                }
                catch (Exception ex)
                {
                    lastError = ex;
                    TryDelete(destination);
                    if (attempt < MaxAttempts)
                    {
                        await Task.Delay(TimeSpan.FromSeconds(attempt * 2), cancellationToken);
                    }
                }
            }

            throw new InvalidOperationException(
                lastError == null ? "Download failed." : lastError.Message,
                lastError);
        }

        public Task<string> ComputeSha256Async(string path, CancellationToken cancellationToken)
        {
            return Task.Run(
                () =>
                {
                    using (var sha = SHA256.Create())
                    using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 20, FileOptions.SequentialScan))
                    {
                        var buffer = new byte[1 << 20];
                        int read;
                        while ((read = stream.Read(buffer, 0, buffer.Length)) > 0)
                        {
                            cancellationToken.ThrowIfCancellationRequested();
                            sha.TransformBlock(buffer, 0, read, null, 0);
                        }

                        sha.TransformFinalBlock(new byte[0], 0, 0);
                        return ToHex(sha.Hash);
                    }
                },
                cancellationToken);
        }

        private static async Task DownloadOnceAsync(
            string url,
            string destination,
            IProgress<DownloadProgressInfo> progress,
            CancellationToken cancellationToken)
        {
            var directory = Path.GetDirectoryName(destination);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            var uri = new Uri(url, UriKind.Absolute);
            if (uri.IsFile)
            {
                await CopyLocalFileAsync(uri.LocalPath, destination, progress, cancellationToken);
                return;
            }

            var request = (HttpWebRequest)WebRequest.Create(uri);
            request.UserAgent = "Marguerite-Downloader/0.2.17";
            request.Timeout = 30_000;
            request.ReadWriteTimeout = 60_000;
            request.AllowAutoRedirect = true;
            request.AutomaticDecompression = DecompressionMethods.GZip | DecompressionMethods.Deflate;
            request.CachePolicy = new RequestCachePolicy(RequestCacheLevel.NoCacheNoStore);

            using (cancellationToken.Register(() => TryAbort(request)))
            using (var response = (HttpWebResponse)await request.GetResponseAsync())
            {
                var total = response.ContentLength;
                var stopwatch = Stopwatch.StartNew();
                long received = 0;
                var buffer = new byte[1 << 16];

                using (var input = response.GetResponseStream())
                using (var output = new FileStream(destination, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 16, true))
                {
                    if (input == null)
                    {
                        throw new IOException("The server returned an empty body.");
                    }

                    int read;
                    while ((read = input.Read(buffer, 0, buffer.Length)) > 0)
                    {
                        cancellationToken.ThrowIfCancellationRequested();
                        output.Write(buffer, 0, read);
                        received += read;

                        var seconds = stopwatch.Elapsed.TotalSeconds;
                        if (progress != null)
                        {
                            progress.Report(new DownloadProgressInfo
                            {
                                Received = received,
                                Total = total,
                                BytesPerSecond = seconds > 0.25 ? received / seconds : 0
                            });
                        }
                    }
                }

                if (total > 0 && received < total)
                {
                    throw new IOException(
                        string.Format("Connection closed early ({0} of {1} bytes).", received, total));
                }
            }
        }

        private static async Task CopyLocalFileAsync(
            string source,
            string destination,
            IProgress<DownloadProgressInfo> progress,
            CancellationToken cancellationToken)
        {
            var total = new FileInfo(source).Length;
            var stopwatch = Stopwatch.StartNew();
            long received = 0;

            using (var input = new FileStream(source, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 16, true))
            using (var output = new FileStream(destination, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 16, true))
            {
                var buffer = new byte[1 << 16];
                int read;
                while ((read = await input.ReadAsync(buffer, 0, buffer.Length, cancellationToken)) > 0)
                {
                    await output.WriteAsync(buffer, 0, read, cancellationToken);
                    received += read;

                    if (progress != null)
                    {
                        var seconds = stopwatch.Elapsed.TotalSeconds;
                        progress.Report(new DownloadProgressInfo
                        {
                            Received = received,
                            Total = total,
                            BytesPerSecond = seconds > 0.15 ? received / seconds : 0
                        });
                    }

                    // Keep a local copy visible as a transfer instead of a
                    // single instantaneous jump.
                    await Task.Delay(4, cancellationToken);
                }
            }
        }

        private static async Task<string> GetStringAsync(string url, CancellationToken cancellationToken)
        {
            Exception lastError = null;

            for (var attempt = 1; attempt <= MaxAttempts; attempt++)
            {
                try
                {
                    var request = (HttpWebRequest)WebRequest.Create(new Uri(url, UriKind.Absolute));
                    request.UserAgent = "Marguerite-Downloader/0.2.17";
                    request.Accept = "application/vnd.github+json";
                    request.Timeout = 20_000;
                    request.AllowAutoRedirect = true;
                    request.CachePolicy = new RequestCachePolicy(RequestCacheLevel.NoCacheNoStore);

                    using (cancellationToken.Register(() => TryAbort(request)))
                    using (var response = (HttpWebResponse)await request.GetResponseAsync())
                    using (var reader = new StreamReader(response.GetResponseStream(), Encoding.UTF8))
                    {
                        return await reader.ReadToEndAsync();
                    }
                }
                catch (OperationCanceledException)
                {
                    throw;
                }
                catch (Exception ex)
                {
                    lastError = ex;
                    if (attempt < MaxAttempts)
                    {
                        await Task.Delay(TimeSpan.FromSeconds(attempt), cancellationToken);
                    }
                }
            }

            throw new InvalidOperationException(
                lastError == null ? "Could not reach the release service." : lastError.Message,
                lastError);
        }

        private static void TryAbort(HttpWebRequest request)
        {
            try
            {
                request.Abort();
            }
            catch
            {
            }
        }

        private static void TryDelete(string path)
        {
            try
            {
                if (File.Exists(path))
                {
                    File.Delete(path);
                }
            }
            catch
            {
            }
        }

        private static string ToHex(byte[] bytes)
        {
            var builder = new StringBuilder(bytes.Length * 2);
            foreach (var b in bytes)
            {
                builder.Append(b.ToString("x2"));
            }

            return builder.ToString();
        }
    }
}
