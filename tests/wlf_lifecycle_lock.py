#!/usr/bin/env python3
"""Check ordinary external exclusive lock access to one WLF file.

Functional validation helper for the WLF lifecycle test. It opens the path
read-write but never writes bytes, attempts one non-blocking exclusive
fcntl.flock and one non-blocking exclusive fcntl.lockf, unlocks each, and closes
the file. Exit 0 when both locks are available, 1 when either is blocked, 2 on
usage or access errors.
"""

import fcntl
import sys


def main():
    if len(sys.argv) != 2:
        print("usage: wlf_lifecycle_lock.py PATH", file=sys.stderr)
        return 2
    path = sys.argv[1]
    try:
        with open(path, "r+b") as stream:
            descriptor = stream.fileno()
            for lock, unlock in (
                (
                    lambda: fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB),
                    lambda: fcntl.flock(descriptor, fcntl.LOCK_UN),
                ),
                (
                    lambda: fcntl.lockf(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB),
                    lambda: fcntl.lockf(descriptor, fcntl.LOCK_UN),
                ),
            ):
                try:
                    lock()
                except BlockingIOError:
                    return 1
                unlock()
    except OSError as error:
        print(f"{path}: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
