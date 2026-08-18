"""Purge stale cargo target directories under the workspace.

Safety design:
- Operates ONLY on an explicit allow-list of directory names passed via argv.
- Every name must start with "target" and must NOT be a protected/in-use dir.
- Each path is resolved and asserted to live directly under the workspace root.
- Build artifacts only: these are fully regenerable by `cargo build`.
- Dry-run by default; requires --apply to actually delete.
"""

import ctypes
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# In-use target dirs that must never be purged by this script.
PROTECTED = {"target", "target_c5"}

_k32 = ctypes.windll.kernel32


def _clear_readonly(path):
    # FILE_ATTRIBUTE_NORMAL = 0x80
    _k32.SetFileAttributesW(ctypes.c_wchar_p(path), 0x80)


def _delete_file(path):
    if _k32.DeleteFileW(ctypes.c_wchar_p(path)):
        return True
    _clear_readonly(path)
    return bool(_k32.DeleteFileW(ctypes.c_wchar_p(path)))


def _remove_dir(path):
    return bool(_k32.RemoveDirectoryW(ctypes.c_wchar_p(path)))


def purge(root_dir):
    """Delete a directory tree via raw Win32 calls. Returns (files, dirs, failures)."""
    files = dirs = 0
    failures = []
    for dirpath, dirnames, filenames in os.walk(root_dir, topdown=False):
        for name in filenames:
            fp = os.path.join(dirpath, name)
            if _delete_file(fp):
                files += 1
            else:
                failures.append((fp, ctypes.GetLastError()))
        for name in dirnames:
            dp = os.path.join(dirpath, name)
            if _remove_dir(dp):
                dirs += 1
            else:
                failures.append((dp, ctypes.GetLastError()))
    if _remove_dir(root_dir):
        dirs += 1
    else:
        failures.append((root_dir, ctypes.GetLastError()))
    return files, dirs, failures


def validate(name):
    if not name.startswith("target"):
        return "not a target dir"
    if name in PROTECTED:
        return "protected / in use"
    full = os.path.join(ROOT, name)
    if os.path.dirname(os.path.abspath(full)) != ROOT:
        return "escapes workspace root"
    if not os.path.isdir(full):
        return "does not exist"
    return None


def main():
    argv = [a for a in sys.argv[1:] if a != "--apply"]
    apply = "--apply" in sys.argv
    if not argv:
        print("usage: purge_stale_targets.py <dir> [<dir>...] [--apply]")
        return 2

    for name in argv:
        err = validate(name)
        if err:
            print("REFUSED %-20s (%s)" % (name, err))
            continue
        full = os.path.join(ROOT, name)
        if not apply:
            print("DRY-RUN would purge %s" % full)
            continue
        files, dirs, failures = purge(full)
        status = "OK" if not failures else "PARTIAL (%d failures)" % len(failures)
        print("%-10s %-20s files=%-7d dirs=%-6d %s" % (status, name, files, dirs, status == "OK" and "" or ""))
        for fp, code in failures[:5]:
            print("    FAIL err=%-4s %s" % (code, fp))
    return 0


if __name__ == "__main__":
    sys.exit(main())
