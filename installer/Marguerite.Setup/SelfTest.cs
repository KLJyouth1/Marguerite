using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;

namespace MargueriteSetup
{
    /// <summary>
    /// Runs without a window and proves the non-destructive half of the
    /// installer contract: payload discovery, the pinned SHA256, the exact
    /// NSIS command line, and the embedded film.
    /// </summary>
    public static class SelfTest
    {
        public static int Run()
        {
            var lines = new List<string>();
            var ok = true;

            try
            {
                var baseDirectory = AppDomain.CurrentDomain.BaseDirectory;
                lines.Add("base=" + baseDirectory);

                var payload = InstallerRunner.FindPayload(baseDirectory);
                if (payload == null)
                {
                    lines.Add("payload=NOT_FOUND");
                    ok = false;
                }
                else
                {
                    var size = new FileInfo(payload.Path).Length;
                    lines.Add("payload=" + payload.Path);
                    lines.Add("payloadSize=" + size);
                    lines.Add("payloadVersion=" + payload.Version);

                    var actual = ComputeSha256(payload.Path);
                    lines.Add("payloadSha256=" + actual);
                    lines.Add("expectedSha256=" + InstallerRunner.ExpectedSha256);
                    var match = string.Equals(actual, InstallerRunner.ExpectedSha256, StringComparison.OrdinalIgnoreCase);
                    lines.Add("payloadHashMatch=" + match);
                    ok &= match;
                }

                var argsWithShortcuts = InstallerRunner.BuildArguments(@"C:\Users\Test User\AppData\Local\Marguerite", true);
                var argsWithoutShortcuts = InstallerRunner.BuildArguments(@"D:\Temp\Marguerite", false);
                lines.Add("argsWithShortcuts=" + argsWithShortcuts);
                lines.Add("argsWithoutShortcuts=" + argsWithoutShortcuts);
                ok &= argsWithShortcuts.EndsWith(" /D=C:\\Users\\Test User\\AppData\\Local\\Marguerite", StringComparison.Ordinal);
                ok &= argsWithoutShortcuts == "/S /NS /D=D:\\Temp\\Marguerite";
                lines.Add("estimatedInstalledBytes=" + InstallerRunner.EstimatedInstalledBytes);

                // 哈希锚点的真值表。旧实现的漏洞是「拿不到哈希指针就放行」，
                // 所以这里逐格证明：只有能拿到可信哈希的格子才返回非 null。
                var matchingSidecar = InstallerRunner.ExpectedSha256 + "  Marguerite.Setup.exe";
                var conflictingSidecar = new string('0', 64);
                ok &= CheckHashAnchor(lines, "pinnedNoSidecar", InstallerRunner.ExpectedVersion, null, InstallerRunner.ExpectedSha256, null);
                ok &= CheckHashAnchor(lines, "pinnedMatchingSidecar", InstallerRunner.ExpectedVersion, matchingSidecar, InstallerRunner.ExpectedSha256, null);
                ok &= CheckHashAnchor(lines, "pinnedConflictingSidecar", InstallerRunner.ExpectedVersion, conflictingSidecar, null, "sidecar_conflicts_with_pin");
                ok &= CheckHashAnchor(lines, "unpinnedSidecar", "9.9.9", conflictingSidecar, conflictingSidecar.Substring(0, 64), null);
                ok &= CheckHashAnchor(lines, "unpinnedNoSidecar", "9.9.9", null, null, "unverifiable");
                ok &= CheckHashAnchor(lines, "unpinnedBlankSidecar", "9.9.9", "   \r\n", null, "unverifiable");
                ok &= CheckHashAnchor(lines, "pinnedBlankSidecar", InstallerRunner.ExpectedVersion, "  \r\n", InstallerRunner.ExpectedSha256, null);

                using (var video = Assembly.GetExecutingAssembly().GetManifestResourceStream("MargueriteSetup.Assets.promo.mp4"))
                {
                    var videoBytes = video == null ? 0 : video.Length;
                    lines.Add("promoVideoBytes=" + videoBytes);
                    ok &= videoBytes > 17000000;
                }
            }
            catch (Exception ex)
            {
                lines.Add("EXCEPTION=" + ex);
                ok = false;
            }

            lines.Add("RESULT=" + (ok ? "PASS" : "FAIL"));

            try
            {
                var directory = Path.Combine(Path.GetTempPath(), "MargueriteSetup");
                Directory.CreateDirectory(directory);
                File.WriteAllText(
                    Path.Combine(directory, "selftest.log"),
                    string.Join(Environment.NewLine, lines.ToArray()) + Environment.NewLine,
                    new UTF8Encoding(false));
            }
            catch
            {
            }

            return ok ? 0 : 1;
        }

        private static bool CheckHashAnchor(
            List<string> lines,
            string name,
            string version,
            string sidecarText,
            string expectedHash,
            string expectedProblem)
        {
            string problem;
            var resolved = InstallerRunner.ResolveExpectedSha256(version, sidecarText, out problem);
            var hashOk = string.Equals(resolved, expectedHash, StringComparison.OrdinalIgnoreCase);
            var problemOk = string.Equals(problem, expectedProblem, StringComparison.Ordinal);
            lines.Add("hashAnchor." + name + "=" + (resolved ?? "NULL") + " problem=" + (problem ?? "NULL"));
            return hashOk && problemOk;
        }

        private static string ComputeSha256(string path)
        {
            using (var sha = SHA256.Create())
            using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 20, FileOptions.SequentialScan))
            {
                var buffer = new byte[1 << 20];
                int read;
                while ((read = stream.Read(buffer, 0, buffer.Length)) > 0)
                {
                    sha.TransformBlock(buffer, 0, read, null, 0);
                }

                sha.TransformFinalBlock(new byte[0], 0, 0);

                var builder = new StringBuilder(sha.Hash.Length * 2);
                foreach (var b in sha.Hash)
                {
                    builder.Append(b.ToString("x2"));
                }

                return builder.ToString();
            }
        }
    }
}
