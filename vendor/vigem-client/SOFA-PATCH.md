Based on CasualX/vigem-client 0.1.4 (MIT, see license.txt).
Only the library sources are vendored; the manifest omits upstream examples and dev dependencies.

The blocking IOCTL wrappers in src/bus.rs now check DeviceIoControl's return
value before calling GetOverlappedResult. An immediate failure other than
ERROR_IO_PENDING must be returned directly: Windows leaves the zero-initialized
OVERLAPPED untouched, which GetOverlappedResult can otherwise interpret as success.
In particular, plugin must reject occupied serial numbers and try the next slot.

XUsbGetUserIndex also checks the returned byte count. Drivers that return no
output must not cause the zero-initialized buffer to be reported as player one.

Upstream: https://github.com/CasualX/vigem-client
