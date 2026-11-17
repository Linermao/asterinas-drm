# File Descriptor and I/O Control

<!--
Put system calls such as
dup, dup2, dup3, fcntl, ioctl, pipe, pipe2, splice, tee, vmsplice, sendfile,
eventfd, eventfd2, memfd_create and fadvise64
under this category.
-->

### `fcntl`

Supported functionality in SCML:

```c
{{#include fcntl.scml}}
```

Unsupported commands:
* `F_NOTIFY`
* `F_OFD_SETLK`, `F_OFD_SETLKW` and `F_OFD_GETLK`
* `F_GETOWN_EX` and `F_SETOWN_EX`
* `F_GETSIG` and `F_SETSIG`
* `F_SETLEASE` and `F_GETLEASE`
* `F_SETPIPE_SZ` and `F_GETPIPE_SZ`
* `F_GET_RW_HINT` and `F_SET_RW_HINT`
* `F_GET_FILE_RW_HINT` and `F_SET_FILE_RW_HINT`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/fcntl.2.html).

### `pipe` and `pipe2`

Supported functionality in SCML:

```c
{{#include pipe_and_pipe2.scml}}
```

Silently-ignored flags:
* `O_DIRECT`
* `O_NONBLOCK`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/pipe.2.html).

### `eventfd` and `eventfd2`

Supported functionality in SCML:

```c
{{#include eventfd_and_eventfd2.scml}}
```

Silently-ignored flags:
* `EFD_NONBLOCK`
* `EFD_SEMAPHORE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/eventfd.2.html).

### `memfd_create`

Supported functionality in SCML:

```c
{{#include memfd_create.scml}}
```

Silently-ignored flags:
* `MFD_HUGETLB`

Unsupported flags:
* `MFD_HUGE_64KB`
* `MFD_HUGE_512KB`
* `MFD_HUGE_1MB`
* `MFD_HUGE_2MB`
* `MFD_HUGE_8MB`
* `MFD_HUGE_16MB`
* `MFD_HUGE_32MB`
* `MFD_HUGE_256MB`
* `MFD_HUGE_512MB`
* `MFD_HUGE_1GB`
* `MFD_HUGE_2GB`
* `MFD_HUGE_16GB`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/memfd_create.2.html).

### `fadvise64`

Supported functionality in SCML:

```c
{{#include fadvise64.scml}}
```

Silently-ignored flags:
* `POSIX_FADV_NORMAL`
* `POSIX_FADV_RANDOM`
* `POSIX_FADV_SEQUENTIAL`
* `POSIX_FADV_WILLNEED`
* `POSIX_FADV_DONTNEED`
* `POSIX_FADV_NOREUSE`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/posix_fadvise.2.html).

### `epoll_ctl`

Supported functionality in SCML:

```c
{{#include epoll_ctl.scml}}
```

Unsupported flags in events:
* `EPOLLEXCLUSIVE`
* `EPOLLWAKEUP`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/epoll_ctl.2.html).

### `poll` and `ppoll`

Supported functionality in SCML:

```c
{{#include poll_and_ppoll.scml}}
```

Unsupported events:
* `POLLRDBAND`
* `POLLWRNORM`
* `POLLWRBAND`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/poll.2.html).

### `ioctl`

Supported functionality in SCML:

```c
{{#include ioctl.scml}}
```

`DRM_IOCTL_MODE_ADDFB2` supports the advertised single-plane pixel formats
with an implicit linear layout (`flags = 0` and zero modifiers).
The first GEM handle, pitch, and offset describe the framebuffer storage.
Multi-plane formats, interlaced framebuffers, and explicit modifiers are not supported.

Legacy `DRM_IOCTL_MODE_PAGE_FLIP` supports synchronous flips and optional
`DRM_MODE_PAGE_FLIP_EVENT` notifications. Async and target-sequence flags are
not supported. Simpledrm copies the image into the firmware framebuffer and
reports completion at the next software vblank; it does not provide hardware
vertical synchronization. DRM file reads return complete events, and polling
reports readability only when an event is queued.

Connectors expose the legacy `DPMS` enum through `MODE_SETPROPERTY` and
`MODE_OBJ_SETPROPERTY`. Simpledrm blanks the framebuffer for the non-On states
because firmware display power control is unavailable.
`MODE_CLOSEFB` releases the calling file's ownership while active planes keep
the framebuffer alive. `WAIT_VBLANK` supports synchronous waits and event
notifications, but not signal delivery.
`MODE_CURSOR` and `MODE_CURSOR2` accept hiding or moving an absent hardware
cursor; installing a cursor buffer is not supported.

DRM PRIME supports exporting CPU-coherent GEM buffers with
`DRM_IOCTL_PRIME_HANDLE_TO_FD` and importing them on the same DRM device with
`DRM_IOCTL_PRIME_FD_TO_HANDLE`.
Exports accept `DRM_CLOEXEC` and `DRM_RDWR`.
Repeated imports into one DRM file return the existing handle.
The dma-buf descriptors support shared memory mappings,
size discovery through `lseek`,
and `DMA_BUF_IOCTL_SYNC` with read/write and start/end flags.
Cross-device DMA attachments and asynchronous fences are not supported.

Evdev devices publish input-class and character-device sysfs links.
Each event node's input parent exports its identity and capability bitmaps in
`uevent` for daemonless device discovery. `EVIOCGPROP` reports an empty
`INPUT_PROP_*` bitmap, matching the current input-device abstraction.

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ioctl.2.html).

### `ioprio_set` and `ioprio_get`

Supported functionality in SCML:

```c
{{#include ioprio_get_and_set.scml}}
```

Unsupported selectors:
* `IOPRIO_WHO_PGRP`
* `IOPRIO_WHO_USER`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/ioprio_set.2.html).
