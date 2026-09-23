# DRM tools

This directory contains small utilities for inspecting and debugging DRM devices.

Open an interactive shell:

```bash
make ENABLE_DRM_TOOLS=true BOOT_PROTOCOL=linux-efi-handover64 run_kernel
```

## kms_resources

`kms_resources` is a read-only KMS diagnostic utility. It prints the resources exposed by a DRM card, including the driver name and version, CRTCs, encoders, connectors, display modes, planes, formats, and object properties.

Then run it inside interactive shell:

```sh
kms_resources
# Or inspect another card:
kms_resources /dev/dri/card1
```

## set_crtc

`set_crtc` creates a dumb framebuffer, configures a connected CRTC, and draws
an animated color pattern. Each rendered frame is submitted with
`DRM_IOCTL_MODE_DIRTYFB` so shadow-buffer updates reach the physical display.

Run it inside the interactive shell:

```sh
set_crtc
# Or use another card:
set_crtc /dev/dri/card1
```
