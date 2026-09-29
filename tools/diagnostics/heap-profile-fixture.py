"""Manual Windows fixture for heap-profile.py (never attach to a production app).

Start this script, attach the profiler to its printed PID, then press Enter.
The ctypes/FFI caller's live bytes should follow:
16384, 32768, 0 (shrink below threshold), 16384, 16384 (failed resize), 0.
Each phase lasts six seconds. The profiler samples every five seconds.
"""
import ctypes
import os
import time
from ctypes import wintypes

kernel = ctypes.WinDLL("kernel32", use_last_error=True)
kernel.HeapCreate.argtypes = [wintypes.DWORD, ctypes.c_size_t, ctypes.c_size_t]
kernel.HeapCreate.restype = wintypes.HANDLE
kernel.HeapAlloc.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_size_t]
kernel.HeapAlloc.restype = ctypes.c_void_p
kernel.HeapReAlloc.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p, ctypes.c_size_t]
kernel.HeapReAlloc.restype = ctypes.c_void_p
kernel.HeapFree.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p]
kernel.HeapFree.restype = wintypes.BOOL
kernel.HeapDestroy.argtypes = [wintypes.HANDLE]
kernel.HeapDestroy.restype = wintypes.BOOL
heap = kernel.HeapCreate(0, 0, 0)
assert heap
print(os.getpid(), flush=True)
input()
block = kernel.HeapAlloc(heap, 0, 16384)
assert block
time.sleep(6)
for size in (32768, 4096, 16384):
    resized = kernel.HeapReAlloc(heap, 0, block, size)
    assert resized
    block = resized
    time.sleep(6)
assert not kernel.HeapReAlloc(heap, 0, block, 1 << 62)
time.sleep(6)
assert kernel.HeapFree(heap, 0, block)
time.sleep(6)
assert kernel.HeapDestroy(heap)
