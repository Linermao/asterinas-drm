#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

source /etc/profile

# Step 1: run dbus
mkdir -p /var/lib/dbus /usr/share/X11/xorg.conf.d
[ -f /var/lib/dbus/machine-id ] || dbus-uuidgen --ensure=/var/lib/dbus/machine-id

if command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

# On a standard NixOS system, /run/opengl-driver is created and populated
# automatically by systemd-tmpfiles during early boot. The tmpfiles rules
# are responsible for setting up volatile runtime paths under /run and
# linking them to the active OpenGL driver closure (graphics-drivers).
#
# In Asterinas, systemd-tmpfiles-setup.service is intentionally masked and
# not executed at boot time. As a result, the /run/opengl-driver hierarchy
# is never created automatically, even though Mesa and the graphics-drivers
# derivation are correctly built and present in the Nix store.
#
# To allow Mesa, GBM, and GLX-based applications (e.g. glxinfo, Xorg) to
# locate their runtime driver files, we manually create a symbolic link
# from /run/opengl-driver to the active graphics-drivers store path here.
#
# This is a temporary workaround for debugging and bring-up. Once
# systemd-tmpfiles support is enabled (or an equivalent init-time mechanism
# is provided), this manual setup should be removed.
DRIVERS_PATH=$(ls -d /nix/store/*-graphics-drivers | head -n1)

if [ -z "$DRIVERS_PATH" ]; then
    echo "graphics-drivers not found"
    exit 1
fi

ln -sf "$DRIVERS_PATH" /run/opengl-driver

mkdir -p /run/user/0
chmod 700 /run/user/0
export XDG_RUNTIME_DIR=/run/user/0

# Step 2: run Xorg
XKB_DATA="/run/current-system/sw/share/X11/xkb"
MODULE_PATH="/run/current-system/sw/lib/xorg/modules"

# Asterinas does not run systemd-tmpfiles at boot, so a stale Xorg lock can
# survive a reboot and make the next Xorg instance mistake display :0 for an
# active server. Only remove the display files when no Xorg process exists.
if ! pgrep -x Xorg >/dev/null 2>&1; then
    rm -f /tmp/.X0-lock /tmp/.X11-unix/X0
fi

nohup Xorg :0 vt1 \
  -modulepath "$MODULE_PATH" \
  -xkbdir "$XKB_DATA" \
  -logverbose 0 \
  -logfile /var/log/xorg_debug.log \
  -novtswitch \
  -keeptty \
  -keyboard keyboard \
  -pointer mouse0 \
  > /var/log/xorg.log 2>&1 &


# Step 3: run xfce4
export DISPLAY=:0

# Keep Xorg on the native VirGL path, then make applications launched by the
# desktop use Zink over the Venus Vulkan device. Restricting the ICD prevents
# Mesa from silently falling back to llvmpipe when the Venus device is usable.
export VK_DRIVER_FILES=/run/opengl-driver/share/vulkan/icd.d/virtio_icd.x86_64.json
export MESA_LOADER_DRIVER_OVERRIDE=zink
export GALLIUM_DRIVER=zink
# The Asterinas X server does not expose explicit DRM format modifiers yet.
# Kopper's DRI2 fallback is required for games to present rendered frames.
export LIBGL_KOPPER_DRI2=true
export MESA_VK_WSI_PRESENT_MODE=immediate

LOG=/var/log/xfce-session.log
mkdir -p "$(dirname "$LOG")"
: > "$LOG"                 # truncate/create
chmod 600 "$LOG"
nohup xfce4-session >>"$LOG" 2>&1 &
