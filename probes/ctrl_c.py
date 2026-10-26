"""Sends Ctrl+C to the console of another process, as the reaper would on Windows.

Usage: ctrl_c.py [--list] <pid>

Exit codes: 0 the event was generated, 2 AttachConsole failed (GetLastError is printed),
3 GenerateConsoleCtrlEvent failed (GetLastError is printed).

With --list nothing is sent: the IDs of all processes attached to that console, except this helper,
are printed as JSON.
"""

import ctypes
import json
import os
import sys
from ctypes import wintypes

CTRL_C_EVENT = 0
MAX_LISTED_PROCESSES = 256


def main(arguments: list[str]) -> int:
    list_only = "--list" in arguments
    pid = int(next(argument for argument in arguments if argument.isdigit()))
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)  # type: ignore[attr-defined]
    kernel32.AttachConsole.argtypes = [wintypes.DWORD]
    kernel32.SetConsoleCtrlHandler.argtypes = [ctypes.c_void_p, wintypes.BOOL]
    kernel32.GenerateConsoleCtrlEvent.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.GetConsoleProcessList.argtypes = [ctypes.POINTER(wintypes.DWORD), wintypes.DWORD]

    kernel32.FreeConsole()  # a process can be attached to only one console
    if not kernel32.AttachConsole(pid):
        print(f"AttachConsole failed: GetLastError={ctypes.get_last_error()}")  # type: ignore[attr-defined]
        return 2
    if list_only:
        buffer = (wintypes.DWORD * MAX_LISTED_PROCESSES)()
        count = kernel32.GetConsoleProcessList(buffer, MAX_LISTED_PROCESSES)
        kernel32.FreeConsole()
        print(json.dumps([process for process in buffer[:count] if process != os.getpid()]))  # not this helper
        return 0
    kernel32.SetConsoleCtrlHandler(None, True)  # this process ignores its own event
    generated = kernel32.GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)
    error = ctypes.get_last_error()  # type: ignore[attr-defined]
    kernel32.FreeConsole()
    if not generated:
        print(f"GenerateConsoleCtrlEvent failed: GetLastError={error}")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
