# NixOS module for the clack client and the udev rules of the board.
#
# The flake gives `self`, so that the default package is the clack package
# of this flake.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.clack;

  # The rules must come before 73-seat-late.rules. That file applies the
  # uaccess tag. A tag after it has no effect.
  rules = pkgs.writeTextFile {
    name = "clack-udev-rules";
    destination = "/lib/udev/rules.d/70-clack.rules";
    text = ''
      # Pico-Relay-B with the clack firmware. The product string is
      # necessary, because other Pico SDK programs on the RP2350 also use
      # the USB ID 2e8a:0009.
      SUBSYSTEM=="tty", ATTRS{idVendor}=="2e8a", ATTRS{idProduct}=="0009", ATTRS{product}=="Pico-Relay-B SCPI", GROUP="${cfg.group}", MODE="0660", TAG+="uaccess"
    ''
    + lib.optionalString cfg.flashing.enable ''
      # RP2350 boot ROM in BOOTSEL mode, for picotool.
      SUBSYSTEM=="usb", ATTR{idVendor}=="2e8a", ATTR{idProduct}=="000f", GROUP="${cfg.group}", MODE="0660", TAG+="uaccess"
      # Raspberry Pi Debug Probe (CMSIS-DAP), for probe-rs.
      SUBSYSTEM=="usb", ATTR{idVendor}=="2e8a", ATTR{idProduct}=="000c", GROUP="${cfg.group}", MODE="0660", TAG+="uaccess"
    '';
  };
in
{
  options.programs.clack = {
    enable = lib.mkEnableOption "the clack client and the udev rules for Pico-Relay-B boards with the clack firmware";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.clack;
      defaultText = lib.literalExpression "clack.packages.\${pkgs.stdenv.hostPlatform.system}.clack";
      description = "The clack package.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "dialout";
      description = ''
        The group that gets access to the boards. The user at the seat also
        gets access through the uaccess tag. Add users to this group for
        access over SSH or from a service.
      '';
    };

    flashing.enable = lib.mkEnableOption ''
      the udev rules for the RP2350 boot ROM (BOOTSEL mode) and the
      Raspberry Pi Debug Probe, and install picotool and probe-rs
    '';
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.package
    ]
    ++ lib.optionals cfg.flashing.enable [
      pkgs.picotool
      pkgs.probe-rs-tools
    ];
    services.udev.packages = [ rules ];
    users.groups.${cfg.group} = { };
  };
}
