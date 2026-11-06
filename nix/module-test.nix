# VM test for the NixOS module. The VM has no board, thus the test checks
# the installed files and the rule syntax, not the USB match.
{ pkgs, module }:
pkgs.testers.runNixOSTest {
  name = "clack-module";
  nodes.machine = {
    imports = [ module ];
    programs.clack = {
      enable = true;
      group = "relays";
      flashing.enable = true;
    };
  };
  testScript = ''
    machine.wait_for_unit("multi-user.target")
    rules = "/etc/udev/rules.d/70-clack.rules"

    machine.succeed("clack --version")
    # Without a board, list finds nothing and stops without an error.
    machine.succeed("clack list")
    machine.succeed("udevadm verify --resolve-names=never " + rules)
    machine.succeed("grep -q 'ATTRS{product}==\"Pico-Relay-B SCPI\"' " + rules)
    machine.succeed("grep -q 'GROUP=\"relays\"' " + rules)
    machine.succeed("grep -q '000f' " + rules)
    machine.succeed("grep -q '000c' " + rules)
    machine.succeed("getent group relays")
    machine.succeed("command -v picotool")
    machine.succeed("command -v probe-rs")
  '';
}
