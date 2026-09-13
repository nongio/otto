# NixOS module: `programs.otto.enable = true;` installs Otto as a login session
# and wires up what the Arch/Debian/RPM packages leave to the distribution —
# the PAM stack for otto-lock, the portal backend, PipeWire, graphics drivers.
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.otto;
in
{
  options.programs.otto = {
    enable = lib.mkEnableOption "Otto, a visually-focused Wayland desktop";

    package = lib.mkPackageOption pkgs "otto" { };

    config = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = "${cfg.package}/share/doc/otto/config.example.toml";
      defaultText = lib.literalExpression ''"''${config.programs.otto.package}/share/doc/otto/config.example.toml"'';
      description = ''
        The system-wide configuration, installed as `/etc/otto/config.toml`.
        Users override it in `~/.config/otto/config.toml`. Set to `null` to
        manage `/etc/otto/config.toml` yourself.
      '';
    };

    extraPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = with pkgs; [ foot ];
      defaultText = lib.literalExpression "with pkgs; [ foot ]";
      description = ''
        Applications installed alongside the desktop. The default dock
        expects a terminal, and `foot` is the one the example configuration
        lists.
      '';
    };

    greeter.enable = lib.mkEnableOption "greetd with Otto's own greeter as the login screen";

    portal.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Register Otto's xdg-desktop-portal backend for screen sharing, screenshots and settings.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ] ++ cfg.extraPackages;
    programs.xwayland.enable = true;

    # The session entry, for any display manager — and under
    # /run/current-system/sw for a greeter started without one.
    services.displayManager.sessionPackages = [ cfg.package ];
    environment.pathsToLink = [ "/share/wayland-sessions" ];

    environment.etc."otto/config.toml" = lib.mkIf (cfg.config != null) {
      source = cfg.config;
    };

    # otto-lock authenticates against a service of its own name and falls
    # through to `other` — deny everything — without one. NixOS builds the
    # stack, fingerprint included when fprintd is on.
    security.pam.services.otto-lock = { };

    hardware.graphics.enable = lib.mkDefault true;
    services.dbus.enable = true;
    security.polkit.enable = lib.mkDefault true;
    services.pipewire = {
      enable = lib.mkDefault true;
      wireplumber.enable = lib.mkDefault true;
    };

    # The chrome draws with fontconfig; the example configuration names Inter,
    # with Noto behind it for the scripts and emoji Inter does not cover.
    fonts.packages = with pkgs; [ inter noto-fonts noto-fonts-color-emoji ];

    # The user unit the portal's D-Bus service file names.
    systemd.packages = lib.mkIf cfg.portal.enable [ cfg.package ];

    xdg.portal = lib.mkIf cfg.portal.enable {
      enable = true;
      extraPortals = [ cfg.package pkgs.xdg-desktop-portal-gtk ];
      # Mirrors components/xdg-desktop-portal-otto/portals.conf.example.
      config.otto = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "otto" ];
        "org.freedesktop.impl.portal.Settings" = [ "otto" ];
        "org.freedesktop.impl.portal.Access" = [ "otto" ];
        "org.freedesktop.impl.portal.Screenshot" = [ "otto" ];
      };
    };

    services.greetd = lib.mkIf cfg.greeter.enable {
      enable = true;
      # Through systemd-cat: greetd hands the greeter the VT as its stderr,
      # so without it a compositor that fails to start leaves no trace.
      settings.default_session = {
        command = "${pkgs.systemd}/bin/systemd-cat --identifier=otto-greeter ${cfg.package}/bin/otto --login";
        user = "greeter";
      };
    };
  };
}
