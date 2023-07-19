{
  description = "ARINC 653 P4 compliant Linux Hypervisor";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    utils.url = "git+https://github.com/numtide/flake-utils.git";
    devshell.url = "github:numtide/devshell";
    fenix = {
      url = "git+https://github.com/nix-community/fenix.git?ref=main";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "git+https://github.com/nix-community/naersk.git";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, utils, fenix, naersk, devshell, ... }@inputs:
    utils.lib.eachSystem [ "x86_64-linux" "i686-linux" "aarch64-linux" ] (system:
      let
        lib = nixpkgs.lib;
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ devshell.overlays.default ];
        };

        # rust target name of the `system`
        rust-target = pkgs.rust.toRustTarget pkgs.pkgsStatic.targetPlatform;

        # converts a string to SHOUT_CASE
        shout = string: builtins.replaceStrings [ "-" ] [ "_" ] (nixpkgs.lib.toUpper string);

        rust-toolchain = with fenix.packages.${system};
          combine [
            latest.rustc
            latest.cargo
            latest.clippy
            latest.rustfmt
            targets.${rust-target}.latest.rust-std
            targets.thumbv6m-none-eabi.latest.rust-std # for no_std test
          ];

        # overrides a naersk-lib which uses the stable toolchain expressed above
        naersk-lib = (naersk.lib.${system}.override {
          cargo = rust-toolchain;
          rustc = rust-toolchain;
        });

        # environment variables to add to the derivations
        env = {
          # environment variable to set the rust target
          CARGO_BUILD_TARGET = rust-target;

          # environment variable to determine the linker suitable for the target
          "CARGO_TARGET_${shout rust-target}_LINKER" =
            let
              inherit (pkgs.pkgsStatic.stdenv) cc;
            in
            "${cc}/bin/${cc.targetPrefix}cc";
        };

        # the provided examples
        examples = [
          {
            name = "hello_part";
            partitions = [ "hello_part" ];
          }
          {
            name = "fuel_tank";
            partitions = [ "fuel_tank_simulation" "fuel_tank_controller" ];
          }
          {
            name = "ping";
            partitions = [ "ping_server" "ping_client" ];
          }
          {
            name = "dev_random";
            partitions = [ "dev_random" ];
          }
          {
            name = "crypto_agility";
            partitions = [
              "crypto_agility_crypto_part"
              "crypto_agility_sender"
              "crypto_agility_receiver"
            ];
          }
        ];

        cargoPackageList = ps: builtins.map (p: "--package=${p}") ps;
      in
      rec {
        packages = {
          # the hypervisor itself
          default = packages.a653rs-linux-hypervisor;
          a653rs-linux-hypervisor = naersk-lib.buildPackage
            rec {
              pname = "a653rs-linux-hypervisor";
              root = ./.;
              cargoBuildOptions = x: x ++ [ "--package" pname ];
              cargoTestOptions = x: x ++ [ "--package" pname ];
            } // env;
        } // (builtins.listToAttrs (builtins.map
          ({ name, partitions }: {
            name = "example-${name}";
            value = naersk-lib.buildPackage
              rec {
                pname = name;
                root = ./.;
                cargoBuildOptions = x: x ++ (cargoPackageList partitions);
                cargoTestOptions = x: x ++ (cargoPackageList partitions);
              } // env;
          }
          )
          examples));

        # a devshell with all the necessary bells and whistles
        devShells.default = (pkgs.devshell.mkShell {
          imports = [ "${devshell}/extra/git/hooks.nix" ];
          name = "a653rs-linux-dev-shell";
          packages = with pkgs; [
            stdenv.cc
            coreutils
            rust-toolchain
            rust-analyzer
            cargo-outdated
            cargo-udeps
            cargo-watch
            cargo-audit
            cargo-expand
            nixpkgs-fmt
            nodePackages.prettier
          ];
          git.hooks = {
            enable = true;
            pre-commit.text = "nix flake check";
          };
          commands = [
            { package = "git-cliff"; }
            { package = "treefmt"; }
            {
              name = "udeps";
              command = ''
                PATH="${fenix.packages.${system}.latest.rustc}/bin:$PATH"
                cargo udeps $@
              '';
              help = pkgs.cargo-udeps.meta.description;
            }
            {
              name = "outdated";
              command = "cargo-outdated outdated";
              help = pkgs.cargo-outdated.meta.description;
            }
            {
              name = "audit";
              command = "cargo audit $@";
              help = pkgs.cargo-audit.meta.description;
            }
            {
              name = "expand";
              command = ''
                PATH="${fenix.packages.${system}.latest.rustc}/bin:$PATH"
                cargo expand $@
              '';
              help = pkgs.cargo-expand.meta.description;
            }
            {
              name = "verify-no_std";
              command = ''
                cd "$PRJ_ROOT"
                cargo build --target thumbv6m-none-eabi --no-default-features
              '';
              help = "Verify that the library builds for no_std without std-features";
              category = "dev";
            }
          ] ++ (
            let
              inherit (builtins) map concatStringsSep;
              inherit (nixpkgs.lib) flatten;
            in
            flatten (map
              ({ name, partitions }: [
                {
                  name = "run-example-${name}";
                  command = ''
                    cd "$PRJ_ROOT"
                    # build partitions
                    cargo build --package ${concatStringsSep " " (cargoPackageList partitions)} --target ${rust-target} --release

                    # (build &) run hypervisor
                    RUST_LOG=''${RUST_LOG:=trace} cargo run --package a653rs-linux-hypervisor --release -- examples/${name}.yaml $@
                  '';
                  help = "Run the ${name} example, consisting of the partitions: ${concatStringsSep "," partitions}";
                  category = "example";
                }
                {
                  name = "systemd-run-example-${name}";
                  command = "systemd-run --user --scope run-example-${name} $@";
                  help = "Run the ${name} example using systemd-run";
                  category = "example";
                }
                {
                  name = "clippy-watch-example-${name}";
                  command = ''
                    cargo watch --exec "clippy ${concatStringsSep " " (cargoPackageList partitions)} --target ${rust-target}"
                  '';
                  help = "Continously clippy the ${name} example";
                  category = "dev";
                }
              ])
              examples)
          );
        });

        # always check these
        checks = {
          nixpkgs-fmt = pkgs.runCommand "nixpkgs-fmt"
            {
              nativeBuildInputs = [ pkgs.nixpkgs-fmt ];
            } "nixpkgs-fmt --check ${./.}; touch $out";
          cargo-fmt = pkgs.runCommand "cargo-fmt"
            {
              nativeBuildInputs = [ rust-toolchain ];
            } "cd ${./.}; cargo fmt --check; touch $out";
        };

        # instructions for the CI server
        hydraJobs = (nixpkgs.lib.filterAttrs (n: _: n != "default") packages) // checks;
      });
}

