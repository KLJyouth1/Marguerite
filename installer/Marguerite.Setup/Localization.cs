using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;

namespace MargueriteSetup
{
    /// <summary>
    /// Two languages are shipped with the setup shell: Simplified Chinese and
    /// English. The lookup is an indexer so XAML can bind to <c>L[Key]</c>;
    /// raising <c>Item[]</c> refreshes every one of those bindings at once.
    /// </summary>
    public sealed class Localization : INotifyPropertyChanged
    {
        private bool _english;

        public Localization()
        {
            if (string.Equals(RuntimeOptions.Language, "en", System.StringComparison.OrdinalIgnoreCase))
            {
                _english = true;
            }
            else if (string.Equals(RuntimeOptions.Language, "zh", System.StringComparison.OrdinalIgnoreCase))
            {
                _english = false;
            }
            else
            {
                _english = CultureInfo.CurrentUICulture.TwoLetterISOLanguageName != "zh";
            }
        }

        public event PropertyChangedEventHandler PropertyChanged;

        public bool IsEnglish
        {
            get { return _english; }
        }

        public string this[string key]
        {
            get
            {
                var table = _english ? En : Zh;
                string value;
                return table.TryGetValue(key, out value) ? value : key;
            }
        }

        public string LanguageButton
        {
            get { return _english ? "中文" : "EN"; }
        }

        public void Toggle()
        {
            _english = !_english;
            var handler = PropertyChanged;
            if (handler != null)
            {
                handler(this, new PropertyChangedEventArgs("Item[]"));
                handler(this, new PropertyChangedEventArgs("LanguageButton"));
            }
        }

        private static readonly Dictionary<string, string> Zh = new Dictionary<string, string>
        {
            { "StepWelcome", "欢迎" },
            { "StepLicense", "许可" },
            { "StepOptions", "选项" },
            { "StepInstalling", "安装" },
            { "StepFinish", "完成" },
            { "StepError", "中断" },
            { "StepOfFormat", "{0} / 05 · {1}" },

            { "SoundMuteTip", "静音" },
            { "SoundOnTip", "开启声音" },
            { "MinimizeTip", "最小化" },
            { "CloseTip", "关闭" },

            { "Start", "开始安装" },
            { "Continue", "继续" },
            { "Back", "返回" },
            { "Install", "安装 Marguerite" },
            { "Retry", "重试" },
            { "Exit", "退出" },
            { "Done", "完成" },
            { "LaunchNow", "启动 Marguerite" },
            { "OpenFolder", "打开安装目录" },
            { "CancelInstall", "取消安装" },
            { "ChooseFile", "选择安装包…" },
            { "ChangePayload", "更换…" },
            { "Ok", "好的" },

            { "WelcomeEyebrow", "WINDOWS · SETUP" },
            { "WelcomeTitle", "让思考，\n成为你的工作台。" },
            { "WelcomeBody", "Marguerite 0.2.19 · Windows x64\n当前用户安装，无需管理员权限。" },
            { "Feature1", "多智能体编排与专家团" },
            { "Feature2", "本地优先的会话与记忆" },
            { "Feature3", "桌面、浏览器与终端一体" },
            { "WelcomeNote", "约 411 MB，内含离线 WebView2 运行时与内置 Python。" },

            { "LicenseEyebrow", "LICENSE · MIT" },
            { "LicenseTitle", "在开始之前" },
            { "LicenseBody", "Marguerite 以 MIT 许可发布。请阅读并同意后继续。" },
            { "LicenseAgree", "我已阅读并同意 MIT 许可协议" },

            { "OptionsEyebrow", "INSTALLATION" },
            { "OptionsTitle", "选择它的位置" },
            { "OptionsBody", "默认安装到当前用户目录；升级会保留现有配置与会话。" },
            { "InstallPathLabel", "安装位置" },
            { "BrowseTitle", "选择安装位置" },
            { "Browse", "浏览…" },
            { "ShortcutsLabel", "创建桌面与开始菜单快捷方式" },
            { "LaunchLabel", "安装完成后启动 Marguerite" },
            { "PayloadLabel", "安装包" },
            { "PayloadReady", "已校验" },
            { "PayloadPending", "安装前校验" },
            { "PayloadMissing", "未找到，点击选择" },
            { "PayloadUnverified", "非 0.2.19 版本 · 需自带校验" },
            { "ExistingInstall", "此位置已有 Marguerite，将继续安装并保留用户数据。" },
            { "OtherInstall", "检测到另一处 Marguerite 安装（{0}）；在新位置安装不会移动或删除旧文件。" },
            { "OptionsNote", "当前用户安装，无需管理员权限。" },
            { "PathInvalid", "请选择一个有效的绝对路径。" },
            { "PathNotWritable", "此目录不可写，请换一个位置。" },

            { "InstallingEyebrow", "INSTALLING" },
            { "InstallingTitle", "正在把 Marguerite\n放进你的桌面。" },
            { "StageVerifying", "正在校验安装包…" },
            { "StagePreparing", "正在准备安装环境…" },
            { "StageExtracting", "正在解压组件…" },
            { "StageWriting", "正在写入程序文件…" },
            { "StageRegistering", "正在注册组件与快捷方式…" },
            { "StageFinalizing", "正在收尾…" },
            { "StageWebView", "首次安装可能需要准备 WebView2 运行时…" },
            { "StageCancelling", "正在取消…" },
            { "InstallingNote", "请保持此窗口打开。" },
            { "ElapsedFormat", "已用时 {0}" },

            { "FinishEyebrow", "COMPLETE" },
            { "FinishTitle", "现在，它是你的了。" },
            { "FinishBody", "Marguerite 0.2.19 已安装到\n{0}" },
            { "FinishNote", "配置与会话数据保存在同一目录；首次启动会自动生成。" },

            { "ErrorEyebrow", "INTERRUPTED" },
            { "ErrorTitle", "安装没有完成。" },
            { "ErrorBody", "{0}" },
            { "LogFormat", "日志：{0}" },

            { "AppRunningTitle", "Marguerite 正在运行" },
            { "AppRunningBody", "覆盖安装前需要先退出正在运行的 Marguerite。你可以自行关闭，或由安装器结束进程。" },
            { "KillAndContinue", "结束进程并继续" },
            { "CloseInstallTitle", "安装仍在进行" },
            { "CloseInstallBody", "现在退出会中断安装，可能留下不完整的文件。确定要退出吗？" },
            { "QuitAnyway", "退出并中断" },
            { "KeepInstalling", "继续安装" },
            { "CancelConfirmTitle", "要取消安装吗？" },
            { "CancelConfirmBody", "取消会中止正在进行的安装，可能留下不完整的文件。" },

            { "PayloadMissingTitle", "需要安装包" },
            { "PayloadMissingBody", "请把 Marguerite_0.2.19_x64-setup.exe 与本安装器放在同一目录，或手动选择它。" },
            { "PayloadInvalidTitle", "这不是有效的安装包" },
            { "PayloadInvalidBody", "请选择名为 Marguerite_<版本>_x64-setup.exe 的文件。" },
            { "PayloadVersionTitle", "版本不匹配" },
            { "PayloadVersionBody", "此安装器为 Marguerite 0.2.19 定制；选择的安装包是 {0}，且没有校验文件。请选择 0.2.19 安装包。" },

            { "DemoBadge", "演示模式 · 不会实际安装" },

            { "ErrPayloadMissing", "未找到 Marguerite 的安装包。" },
            { "ErrPayloadHash", "安装包校验失败。\n预期 {0}\n实际 {1}" },
            { "ErrExitCode", "安装程序返回错误代码 {0}。" },
            { "ErrMainBinaryMissing", "安装程序报告成功，但没有找到主程序。" },
            { "ErrSpawn", "无法启动安装程序：{0}" },
            { "ErrPayloadVersion", "此安装器仅验证 0.2.19；所选安装包版本为 {0}。" },
            { "ErrUnknown", "安装失败：{0}" },
        };

        private static readonly Dictionary<string, string> En = new Dictionary<string, string>
        {
            { "StepWelcome", "Welcome" },
            { "StepLicense", "License" },
            { "StepOptions", "Options" },
            { "StepInstalling", "Installing" },
            { "StepFinish", "Finish" },
            { "StepError", "Interrupted" },
            { "StepOfFormat", "{0} / 05 · {1}" },

            { "SoundMuteTip", "Mute" },
            { "SoundOnTip", "Sound on" },
            { "MinimizeTip", "Minimize" },
            { "CloseTip", "Close" },

            { "Start", "Install" },
            { "Continue", "Continue" },
            { "Back", "Back" },
            { "Install", "Install Marguerite" },
            { "Retry", "Try again" },
            { "Exit", "Exit" },
            { "Done", "Done" },
            { "LaunchNow", "Launch Marguerite" },
            { "OpenFolder", "Open folder" },
            { "CancelInstall", "Cancel installation" },
            { "ChooseFile", "Choose installer…" },
            { "ChangePayload", "Change…" },
            { "Ok", "OK" },

            { "WelcomeEyebrow", "WINDOWS · SETUP" },
            { "WelcomeTitle", "A quieter way\nto think and build." },
            { "WelcomeBody", "Marguerite 0.2.19 · Windows x64\nInstalls for the current user. No admin required." },
            { "Feature1", "Multi-agent orchestration" },
            { "Feature2", "Local-first sessions and memory" },
            { "Feature3", "Desktop, browser and terminal in one" },
            { "WelcomeNote", "About 411 MB, including the offline WebView2 runtime and a bundled Python." },

            { "LicenseEyebrow", "LICENSE · MIT" },
            { "LicenseTitle", "Before we begin" },
            { "LicenseBody", "Marguerite is released under the MIT license. Please read and agree to continue." },
            { "LicenseAgree", "I have read and agree to the MIT license" },

            { "OptionsEyebrow", "INSTALLATION" },
            { "OptionsTitle", "Choose where it lives" },
            { "OptionsBody", "Installs for the current user. Upgrades keep your settings and sessions." },
            { "InstallPathLabel", "Install location" },
            { "BrowseTitle", "Choose install location" },
            { "Browse", "Browse…" },
            { "ShortcutsLabel", "Create desktop and Start menu shortcuts" },
            { "LaunchLabel", "Launch Marguerite after installation" },
            { "PayloadLabel", "Installer payload" },
            { "PayloadReady", "Verified" },
            { "PayloadPending", "Verified before install" },
            { "PayloadMissing", "Not found — choose file" },
            { "PayloadUnverified", "Non-0.2.19 · sidecar hash required" },
            { "ExistingInstall", "An existing install was found here; user data will be kept." },
            { "OtherInstall", "Another Marguerite installation was found at {0}; installing here will not move or remove the old files." },
            { "OptionsNote", "Per-user install. No admin required." },
            { "PathInvalid", "Choose a valid absolute path." },
            { "PathNotWritable", "This folder is not writable." },

            { "InstallingEyebrow", "INSTALLING" },
            { "InstallingTitle", "Installing Marguerite." },
            { "StageVerifying", "Verifying the installer…" },
            { "StagePreparing", "Preparing the installation…" },
            { "StageExtracting", "Extracting components…" },
            { "StageWriting", "Writing program files…" },
            { "StageRegistering", "Registering components…" },
            { "StageFinalizing", "Finishing up…" },
            { "StageWebView", "First install may prepare the WebView2 runtime…" },
            { "StageCancelling", "Cancelling…" },
            { "InstallingNote", "Please keep this window open." },
            { "ElapsedFormat", "Elapsed {0}" },

            { "FinishEyebrow", "COMPLETE" },
            { "FinishTitle", "It's yours now." },
            { "FinishBody", "Marguerite 0.2.19 is installed at\n{0}" },
            { "FinishNote", "Settings and sessions live in the same folder." },

            { "ErrorEyebrow", "INTERRUPTED" },
            { "ErrorTitle", "Installation didn't finish." },
            { "ErrorBody", "{0}" },
            { "LogFormat", "Log: {0}" },

            { "AppRunningTitle", "Marguerite is running" },
            { "AppRunningBody", "Close Marguerite before installing over it. You can quit it yourself or let Setup end the process." },
            { "KillAndContinue", "End process and continue" },
            { "CloseInstallTitle", "Installation is running" },
            { "CloseInstallBody", "Quitting now interrupts installation and may leave incomplete files." },
            { "QuitAnyway", "Quit anyway" },
            { "KeepInstalling", "Keep installing" },
            { "CancelConfirmTitle", "Cancel the installation?" },
            { "CancelConfirmBody", "Cancelling stops the installation and may leave incomplete files." },

            { "PayloadMissingTitle", "Installer payload required" },
            { "PayloadMissingBody", "Place Marguerite_0.2.19_x64-setup.exe next to this Setup, or choose it manually." },
            { "PayloadInvalidTitle", "Not a valid installer" },
            { "PayloadInvalidBody", "Choose a file named Marguerite_<version>_x64-setup.exe." },
            { "PayloadVersionTitle", "Version mismatch" },
            { "PayloadVersionBody", "This setup is built for Marguerite 0.2.19; the selected payload is {0} and has no checksum sidecar." },

            { "DemoBadge", "Demo mode · no changes to your system" },

            { "ErrPayloadMissing", "The Marguerite installer was not found." },
            { "ErrPayloadHash", "Installer verification failed.\nExpected {0}\nActual {1}" },
            { "ErrExitCode", "The installer returned error code {0}." },
            { "ErrMainBinaryMissing", "The installer reported success, but the main executable is missing." },
            { "ErrSpawn", "Could not start the installer: {0}" },
            { "ErrPayloadVersion", "This setup only verifies 0.2.19; the selected payload is {0}." },
            { "ErrUnknown", "Installation failed: {0}" },
        };
    }
}
