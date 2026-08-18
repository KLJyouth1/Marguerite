"""Report free disk space on D: and the size of each target* dir under the workspace."""
import ctypes
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def free_space(drive):
    k = ctypes.windll.kernel32
    avail = ctypes.c_ulonglong(0)
    total = ctypes.c_ulonglong(0)
    free = ctypes.c_ulonglong(0)
    k.GetDiskFreeSpaceExW(
        ctypes.c_wchar_p(drive),
        ctypes.byref(avail),
        ctypes.byref(total),
        ctypes.byref(free),
    )
    return total.value, avail.value


def dir_size(path):
    total = 0
    for dirpath, _dirnames, filenames in os.walk(path):
        for name in filenames:
            fp = os.path.join(dirpath, name)
            try:
                total += os.path.getsize(fp)
            except OSError:
                pass
    return total


def main():
    total, avail = free_space("D:\\")
    print("D: total %.1f GB  free %.1f GB" % (total / 1e9, avail / 1e9))
    print("--- target dirs under %s ---" % ROOT)
    grand = 0
    for name in sorted(os.listdir(ROOT)):
        full = os.path.join(ROOT, name)
        if not os.path.isdir(full):
            continue
        if not (name.startswith("target") or name.startswith("Downloads")):
            continue
        size = dir_size(full)
        grand += size
        print("  %-40s %8.2f GB" % (name, size / 1e9))
    print("  %-40s %8.2f GB" % ("TOTAL reclaimable", grand / 1e9))


if __name__ == "__main__":
    main()
