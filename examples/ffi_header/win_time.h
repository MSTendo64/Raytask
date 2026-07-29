/* RayTask FFI header — no gcc required.
 * Prototypes are parsed by RayTask and bound to kernel32.dll.
 */
#pragma once

typedef unsigned int DWORD;
typedef unsigned long ULONG;

__declspec(dllimport) DWORD GetTickCount(void);
DWORD GetCurrentProcessId(void);
