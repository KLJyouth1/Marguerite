using System;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using WinForms = System.Windows.Forms;

namespace MargueriteSetup
{
    /// <summary>
    /// Vista-style folder picker (IFileDialog + FOS_PICKFOLDERS) so the browse
    /// action does not fall back to the dated SHBrowseForFolder tree. Any COM
    /// failure falls back to the framework dialog rather than breaking install.
    /// </summary>
    internal static class FolderPicker
    {
        private const uint FosNoChangeDir = 0x00000008;
        private const uint FosPickFolders = 0x00000020;
        private const uint FosForceFileSystem = 0x00000040;
        private const uint FosPathMustExist = 0x00000800;
        private const uint SigdnFileSystemPath = 0x80058000;

        private static readonly Guid ClsidFileOpenDialog = new Guid("DC1C5A9C-E88A-4DDE-A5A1-60F82A20AEF7");
        private static readonly Guid IidFileDialog = new Guid("42F85136-DB7E-439C-85F1-E4075D135FC8");
        private static readonly Guid IidShellItem = new Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE");

        public static string Pick(Window owner, string title, string initialPath)
        {
            bool shown;
            var result = TryPickModern(owner, title, initialPath, out shown);
            return shown ? result : PickLegacy(title, initialPath);
        }

        private static string TryPickModern(Window owner, string title, string initialPath, out bool shown)
        {
            shown = false;
            IFileDialog fileDialog = null;
            IShellItem result = null;

            try
            {
                var clsid = ClsidFileOpenDialog;
                var iid = IidFileDialog;
                var hr = CoCreateInstance(ref clsid, IntPtr.Zero, 1, ref iid, out fileDialog);
                if (hr != 0 || fileDialog == null)
                {
                    return null;
                }

                uint options;
                fileDialog.GetOptions(out options);
                fileDialog.SetOptions(
                    options
                    | FosPickFolders
                    | FosForceFileSystem
                    | FosPathMustExist
                    | FosNoChangeDir);
                fileDialog.SetTitle(title);

                if (!string.IsNullOrWhiteSpace(initialPath))
                {
                    IShellItem folder = null;
                    try
                    {
                        var shellItemId = IidShellItem;
                        if (SHCreateItemFromParsingName(initialPath, IntPtr.Zero, ref shellItemId, out folder) == 0
                            && folder != null)
                        {
                            fileDialog.SetFolder(folder);
                        }
                    }
                    catch
                    {
                        // A non-existent initial folder is harmless.
                    }
                    finally
                    {
                        if (folder != null)
                        {
                            Marshal.ReleaseComObject(folder);
                        }
                    }
                }

                shown = true;
                hr = fileDialog.Show(new WindowInteropHelper(owner).Handle);
                if (hr != 0)
                {
                    return null;
                }

                hr = fileDialog.GetResult(out result);
                if (hr != 0 || result == null)
                {
                    return null;
                }

                IntPtr namePointer;
                hr = result.GetDisplayName(SigdnFileSystemPath, out namePointer);
                if (hr != 0 || namePointer == IntPtr.Zero)
                {
                    return null;
                }

                try
                {
                    return Marshal.PtrToStringUni(namePointer);
                }
                finally
                {
                    Marshal.FreeCoTaskMem(namePointer);
                }
            }
            catch
            {
                shown = false;
                return null;
            }
            finally
            {
                if (result != null)
                {
                    Marshal.ReleaseComObject(result);
                }

                if (fileDialog != null)
                {
                    Marshal.ReleaseComObject(fileDialog);
                }
            }
        }

        private static string PickLegacy(string title, string initialPath)
        {
            using (var dialog = new WinForms.FolderBrowserDialog())
            {
                dialog.Description = title;
                dialog.ShowNewFolderButton = true;
                if (!string.IsNullOrWhiteSpace(initialPath) && System.IO.Directory.Exists(initialPath))
                {
                    dialog.SelectedPath = initialPath;
                }

                return dialog.ShowDialog() == WinForms.DialogResult.OK ? dialog.SelectedPath : null;
            }
        }

        [DllImport("ole32.dll")]
        private static extern int CoCreateInstance(
            ref Guid clsid,
            IntPtr outer,
            uint context,
            ref Guid iid,
            [MarshalAs(UnmanagedType.Interface)] out IFileDialog dialog);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        private static extern int SHCreateItemFromParsingName(
            [MarshalAs(UnmanagedType.LPWStr)] string path,
            IntPtr bindingContext,
            ref Guid iid,
            [MarshalAs(UnmanagedType.Interface)] out IShellItem item);

        [ComImport]
        [Guid("42F85136-DB7E-439C-85F1-E4075D135FC8")]
        [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        private interface IFileDialog
        {
            [PreserveSig]
            int Show(IntPtr parent);

            [PreserveSig]
            int SetFileTypes(uint count, IntPtr filterSpec);

            [PreserveSig]
            int SetFileTypeIndex(uint index);

            [PreserveSig]
            int GetFileTypeIndex(out uint index);

            [PreserveSig]
            int Advise(IntPtr events, out uint cookie);

            [PreserveSig]
            int Unadvise(uint cookie);

            [PreserveSig]
            int SetOptions(uint options);

            [PreserveSig]
            int GetOptions(out uint options);

            [PreserveSig]
            int SetDefaultFolder(IShellItem item);

            [PreserveSig]
            int SetFolder(IShellItem item);

            [PreserveSig]
            int GetFolder(out IShellItem item);

            [PreserveSig]
            int GetCurrentSelection(out IShellItem item);

            [PreserveSig]
            int SetFileName([MarshalAs(UnmanagedType.LPWStr)] string name);

            [PreserveSig]
            int GetFileName([MarshalAs(UnmanagedType.LPWStr)] out string name);

            [PreserveSig]
            int SetTitle([MarshalAs(UnmanagedType.LPWStr)] string title);

            [PreserveSig]
            int SetOkButtonLabel([MarshalAs(UnmanagedType.LPWStr)] string text);

            [PreserveSig]
            int SetFileNameLabel([MarshalAs(UnmanagedType.LPWStr)] string label);

            [PreserveSig]
            int GetResult(out IShellItem item);

            [PreserveSig]
            int AddPlace(IShellItem item, int fdap);

            [PreserveSig]
            int SetDefaultExtension([MarshalAs(UnmanagedType.LPWStr)] string extension);

            [PreserveSig]
            int Close(int hr);

            [PreserveSig]
            int SetClientGuid(ref Guid guid);

            [PreserveSig]
            int ClearClientData();

            [PreserveSig]
            int SetFilter(IntPtr filter);
        }

        [ComImport]
        [Guid("43826D1E-E718-42EE-BC55-A1E261C37BFE")]
        [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        private interface IShellItem
        {
            [PreserveSig]
            int BindToHandler(IntPtr context, ref Guid handler, ref Guid iid, out IntPtr result);

            [PreserveSig]
            int GetParent(out IShellItem parent);

            [PreserveSig]
            int GetDisplayName(uint format, out IntPtr name);

            [PreserveSig]
            int GetAttributes(uint mask, out uint attributes);

            [PreserveSig]
            int Compare(IShellItem item, uint hint, out int order);
        }
    }
}
