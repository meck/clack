{
  description = "clack: SCPI relay controller firmware and client for the Waveshare Pico-Relay-B with a Raspberry Pi Pico 2 (RP2350)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      # Import nixpkgs and make the build set only one time for each system.
      # All outputs then use the same instances.
      pkgsFor = nixpkgs.lib.genAttrs systems (
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        }
      );
      buildFor = nixpkgs.lib.genAttrs systems (system: build pkgsFor.${system});
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f pkgsFor.${system});

      chip = "RP235x";

      build =
        pkgs:
        let
          inherit (pkgs) lib;
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchain);

          # Build in a subdirectory. The firmware and the client find
          # ../relay-scpi there. Each crate gets only relay-scpi and its own
          # directory, thus a change in one crate does not rebuild the other.
          # The firmware needs files that crane's default filter removes,
          # for example memory.x and .cargo/config.toml.
          inDir = dir: {
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./relay-scpi
                ./${dir}
              ];
            };
            cargoLock = ./${dir}/Cargo.lock;
            cargoToml = ./${dir}/Cargo.toml;
            postUnpack = ''
              cd $sourceRoot/${dir}
              sourceRoot="."
            '';
          };

          scpiArgs = inDir "relay-scpi" // {
            pname = "relay-scpi";
            version = (craneLib.crateNameFromCargoToml { cargoToml = ./relay-scpi/Cargo.toml; }).version;
            strictDeps = true;
          };
          # Build the dependencies of all features, so that the clippy check
          # with --all-features also uses the cache.
          scpiDeps = craneLib.buildDepsOnly (scpiArgs // { cargoExtraArgs = "--locked --all-features"; });

          # The base arguments are sufficient for the rustfmt check.
          firmwareBase = inDir "firmware" // {
            pname = "clack-firmware";
            version = (craneLib.crateNameFromCargoToml { cargoToml = ./firmware/Cargo.toml; }).version;
            strictDeps = true;
          };
          # The dummy source of buildDepsOnly cannot link for the RP2350,
          # thus the firmware builds without a separate dependency step.
          firmwareArgs = firmwareBase // {
            cargoArtifacts = null;
            doCheck = false;
            # The host strip removes the debug data that defmt and probe-rs
            # use. patchelf has no function on a static ARM image.
            dontStrip = true;
            dontPatchELF = true;
            nativeBuildInputs = [
              pkgs.flip-link
              pkgs.picotool
            ];
          };

          firmware = craneLib.buildPackage (
            firmwareArgs
            // {
              postInstall = ''
                mv $out/bin/clack-firmware $out/bin/clack.elf
                picotool uf2 convert $out/bin/clack.elf -t elf \
                  $out/clack.uf2 -t uf2 --family rp2350-arm-s
              '';
            }
          );

          # The serialport crate reads the USB data of the ports with
          # libudev on Linux. macOS uses IOKit from the SDK.
          clientNativeInputs = [ pkgs.pkg-config ];
          clientInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.udev ];

          clackArgs = inDir "cli" // {
            pname = "clack";
            version = (craneLib.crateNameFromCargoToml { cargoToml = ./cli/Cargo.toml; }).version;
            strictDeps = true;
            nativeBuildInputs = clientNativeInputs;
            buildInputs = clientInputs;
          };
          clackDeps = craneLib.buildDepsOnly clackArgs;
          clack = craneLib.buildPackage (
            clackArgs
            // {
              cargoArtifacts = clackDeps;
              meta.mainProgram = "clack";
            }
          );
        in
        {
          inherit
            toolchain
            craneLib
            firmware
            scpiArgs
            scpiDeps
            firmwareBase
            firmwareArgs
            clackArgs
            clackDeps
            clack
            clientNativeInputs
            clientInputs
            ;
        };
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          b = buildFor.${pkgs.stdenv.hostPlatform.system};
        in
        {
          default = b.firmware;
          firmware = b.firmware;
          clack = b.clack;
        }
      );

      checks = forAllSystems (
        pkgs:
        let
          b = buildFor.${pkgs.stdenv.hostPlatform.system};
        in
        {
          firmware = b.firmware;
          relay-scpi-test = b.craneLib.cargoTest (b.scpiArgs // { cargoArtifacts = b.scpiDeps; });
          relay-scpi-clippy = b.craneLib.cargoClippy (
            b.scpiArgs
            // {
              cargoArtifacts = b.scpiDeps;
              # The firmware uses the defmt feature, thus check it too.
              cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
            }
          );
          relay-scpi-fmt = b.craneLib.cargoFmt b.scpiArgs;
          firmware-fmt = b.craneLib.cargoFmt b.firmwareBase;
          firmware-clippy = b.craneLib.cargoClippy (
            b.firmwareArgs
            // {
              cargoClippyExtraArgs = "-- --deny warnings";
            }
          );
          # The package build also runs the client tests.
          clack = b.clack;
          clack-clippy = b.craneLib.cargoClippy (
            b.clackArgs
            // {
              cargoArtifacts = b.clackDeps;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            }
          );
          clack-fmt = b.craneLib.cargoFmt b.clackArgs;
        }
        # The NixOS VM test runs only on Linux.
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          nixos-module = import ./nix/module-test.nix {
            inherit pkgs;
            module = self.nixosModules.default;
          };
        }
      );

      apps = forAllSystems (
        pkgs:
        let
          b = buildFor.${pkgs.stdenv.hostPlatform.system};
          flash = pkgs.writeShellApplication {
            name = "clack-flash";
            runtimeInputs = [ pkgs.probe-rs-tools ];
            # The arguments go to probe-rs, for example --probe 2e8a:000c.
            # One command writes the flash and resets the chip. Thus
            # probe-rs asks for a probe only one time.
            text = ''
              probe-rs download --chip ${chip} --reset "$@" ${b.firmware}/bin/clack.elf
            '';
          };
          flash-uf2 = pkgs.writeShellApplication {
            name = "clack-flash-uf2";
            runtimeInputs = [ pkgs.picotool ];
            text = ''
              picotool load -v -x ${b.firmware}/clack.uf2
            '';
          };
          clackApp = {
            type = "app";
            program = "${b.clack}/bin/clack";
            meta.description = "Control the relays of the connected boards";
          };
        in
        {
          default = clackApp;
          clack = clackApp;
          flash = {
            type = "app";
            program = "${flash}/bin/clack-flash";
            meta.description = "Flash the firmware with a debug probe (probe-rs)";
          };
          flash-uf2 = {
            type = "app";
            program = "${flash-uf2}/bin/clack-flash-uf2";
            meta.description = "Flash the firmware to a board in BOOTSEL mode (picotool)";
          };
        }
      );

      devShells = forAllSystems (
        pkgs:
        let
          b = buildFor.${pkgs.stdenv.hostPlatform.system};
        in
        {
          default = pkgs.mkShell {
            buildInputs = b.clientInputs;
            packages = b.clientNativeInputs ++ [
              b.clack
              b.toolchain
              pkgs.flip-link
              pkgs.probe-rs-tools
              pkgs.picotool
              pkgs.picocom
              (pkgs.python3.withPackages (ps: [
                ps.pyvisa
                ps.pyvisa-py
                ps.pyserial
              ]))
            ];
          };
        }
      );

      # programs.clack: the client and the udev rules for the boards.
      nixosModules = {
        default = self.nixosModules.clack;
        clack = import ./nix/module.nix self;
      };

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
