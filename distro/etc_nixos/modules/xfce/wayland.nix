# SPDX-License-Identifier: MPL-2.0

{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.xfce-wayland;

  # startxfce4 discovers Labwc through PATH and supplies its Xfce-specific config.
  debugLabwc = pkgs.writeShellScriptBin "labwc" ''
    exec ${pkgs.aster-labwc}/bin/labwc --debug "$@"
  '';

  startWayland = pkgs.writeShellScriptBin "start_xfce_wayland" ''
    set -eu
    umask 077
    exec >> /var/log/xfce-wayland.log 2>&1

    # Match the existing root desktop's system-wide application environment.
    # Profiles may reference unset variables or use nonzero statuses as conditions.
    set +eu
    . /etc/profile
    set -eu
    export PATH=${lib.makeBinPath [
      debugLabwc
      pkgs.xfce4-session
      pkgs.xfce4-terminal
      pkgs.dbus
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.gnused
      pkgs.gawk
    ]}:"$PATH"

    export HOME=/root
    mkdir -p /run/user/0 /var/lib/dbus
    chmod 700 /run/user/0
    export XDG_RUNTIME_DIR=/run/user/0
    dbus-uuidgen --ensure=/var/lib/dbus/machine-id

    # Always exercise native DRM, even when invoked from an existing desktop.
    unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SESSION_MANAGER
    unset DBUS_SESSION_BUS_ADDRESS XFCE4_SESSION_COMPOSITOR
    export XDG_SESSION_TYPE=wayland
    export XDG_CURRENT_DESKTOP=XFCE
    export XDG_SESSION_DESKTOP=xfce
    export GDK_BACKEND=wayland
    export QT_QPA_PLATFORM=wayland
    export XKB_CONFIG_ROOT=${pkgs.xkeyboard-config}/share/X11/xkb

    # This root service has no logind login session.
    export LIBSEAT_BACKEND=seatd
    export LIBSEAT_LOGLEVEL=debug
    export WLR_BACKENDS=${if cfg.enableInput then "drm,libinput" else "drm"}
    # Keep display bring-up possible before input device discovery is complete.
    export WLR_LIBINPUT_NO_DEVICES=1
    export WLR_DRM_DEVICES=${lib.escapeShellArg cfg.drmDevice}
    export WLR_RENDERER=pixman
    export WLR_DRM_NO_ATOMIC=1
    export WLR_DRM_NO_MODIFIERS=1
    export WLR_NO_HARDWARE_CURSORS=1
    export WLR_SCENE_DISABLE_DIRECT_SCANOUT=1

    echo "Starting ${cfg.session} on Labwc ($(date -Is))"
    ${
      if cfg.session == "xfce" then
        ''exec dbus-run-session -- startxfce4 --wayland''
      else
        ''exec dbus-run-session -- labwc --session "xfce4-terminal --disable-server"''
    }
  '';
in
{
  options.services.xfce-wayland = {
    enable = lib.mkEnableOption "the experimental root Xfce Wayland desktop on Asterinas";

    session = lib.mkOption {
      type = lib.types.enum [
        "xfce"
        "terminal"
      ];
      default = "xfce";
      description = "Start the full Xfce session or a single native Wayland terminal.";
    };

    enableInput = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Initialize libinput alongside DRM. Disable to isolate display initialization.";
    };

    drmDevice = lib.mkOption {
      type = lib.types.str;
      default = "/dev/dri/card0";
      description = "DRM device passed to the wlroots backend. Explicit selection may still require sysfs.";
    };
  };

  config = lib.mkIf cfg.enable {
    nixpkgs.overlays = [ (import ../../overlays/desktop/wayland.nix) ];

    services.xserver.enable = lib.mkDefault true;
    services.xserver.desktopManager.xfce = {
      enable = true;
      enableWaylandSession = true;
      enableScreensaver = lib.mkDefault false;
    };

    programs.labwc = {
      enable = true;
      package = pkgs.aster-labwc;
    };

    services.seatd = {
      enable = true;
      logLevel = "debug";
    };

    # The notify handshake can leave seatd activating on Asterinas even while
    # its process is running. Wait for the socket before releasing dependents.
    systemd.services.seatd.serviceConfig = {
      Type = lib.mkForce "simple";
      NotifyAccess = lib.mkForce "none";
      ExecStart = lib.mkForce "${pkgs.seatd.bin}/bin/seatd -u ${config.services.seatd.user} -g ${config.services.seatd.group} -l ${config.services.seatd.logLevel}";
      ExecStartPost = pkgs.writeShellScript "wait-for-seatd-socket" ''
        set -eu
        until [ -S /run/seatd.sock ]; do
          ${pkgs.coreutils}/bin/sleep 0.1
        done
      '';
      TimeoutStartSec = 15;
    };

    environment.systemPackages = [
      startWayland
      pkgs.wayland-utils
      pkgs.aster-wayland-libinput.bin
    ];

    # The existing module still supplies Xfce packages, wallpaper and X11 fallback.
    systemd.services.xfce-desktop.enable = false;

    systemd.services.xfce-wayland = {
      description = "Experimental Xfce Wayland Desktop";
      wantedBy = [ "multi-user.target" ];
      requires = [ "seatd.service" ];
      after = [ "seatd.service" ];
      conflicts = [ "getty@tty1.service" ];
      serviceConfig = {
        Type = "simple";
        User = "root";
        Environment = [ "HOME=/root" ];
        ExecStart = "${startWayland}/bin/start_xfce_wayland";
        StandardInput = "tty";
        StandardOutput = "journal";
        StandardError = "journal";
        TTYPath = "/dev/tty1";
        KillMode = "control-group";
        # Keep the first failure visible instead of repeatedly retrying it.
        Restart = "no";
      };
    };
  };
}
