# SPDX-License-Identifier: MPL-2.0

final: prev:
{
  # Scope the daemonless device library to the experimental Wayland stack.
  # libudev-zero still requires the sysfs interfaces used by its callers.
  aster-wayland-libinput = prev.libinput.override {
    udev = final.libudev-zero;
    wacomSupport = false;
  };

  aster-labwc =
    (prev.labwc.override {
      libinput = final.aster-wayland-libinput;
      # Match the wlroots ABI used by Labwc in the pinned nixpkgs.
      wlroots_0_19 =
        (prev.wlroots_0_19.override {
          libinput = final.aster-wayland-libinput;
        }).overrideAttrs
          (oldAttrs: {
            # Asterinas returns ENOTTY for the unimplemented CREATE_LEASE ioctl.
            # Use wlroots' existing reopen/authenticate fallback in that case.
            postPatch = (oldAttrs.postPatch or "") + ''
              substituteInPlace render/allocator/allocator.c \
                --replace-fail \
                'lease_fd != -EINVAL && lease_fd != -EOPNOTSUPP' \
                'lease_fd != -EINVAL && lease_fd != -EOPNOTSUPP && lease_fd != -ENOTTY'
            '';
          });
    }).overrideAttrs
      (oldAttrs: {
        # Run native Wayland clients during the initial compatibility probe.
        mesonFlags = builtins.filter (
          flag: !(prev.lib.hasPrefix "-Dxwayland=" flag)
        ) (oldAttrs.mesonFlags or [ ]) ++ [ "-Dxwayland=disabled" ];
      });
}
