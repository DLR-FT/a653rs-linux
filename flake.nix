{
  description = "ARINC 653 P4 compliant Linux Hypervisor";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    utils.url = "git+https://github.com/numtide/flake-utils.git";
    devshell.url = "github:numtide/devshell";
    sel4-utils = {
      url = "github:DLR-FT/seL4-nix-utils";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "utils";
    };
    fenix = {
      url = "git+https://github.com/nix-community/fenix.git?ref=main";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "git+https://github.com/nix-community/naersk.git";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { self, nixpkgs, utils, naersk, devshell, ... }@inputs:
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

        # Rust distribution for our hostSystem
        fenix = inputs.fenix.packages.${system};

        rust-toolchain = with fenix;
          combine [
            stable.rustc
            stable.cargo
            stable.clippy
            latest.rustfmt
            targets.${rust-target}.stable.rust-std
            targets.thumbv6m-none-eabi.stable.rust-std # for no_std test
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
            name = "hello_part_no_macros";
            partitions = [ "hello_part_no_macros" ];
          }
          {
            name = "redirect_stdio";
            partitions = [ "redirect_stdio" ];
            preRun = ''
              touch $PRJ_ROOT/std{out,err}
              echo $'hello\nworld!\n' > $PRJ_ROOT/stdin
            '';
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
            name = "ping_queue";
            partitions = [ "ping_queue_server" "ping_queue_client" ];
          }
        ];

        cargoPackageList = ps: builtins.map (p: "--package=${p}") ps;
      in
      rec {
        packages = {
          # the hypervisor itself
          default = packages.a653rs-linux-hypervisor;
          minimal-linux-example = inputs.sel4-utils.packages.x86_64-linux.linux-aarch64.override {
            extraRootfsFiles = {
              "/bin/hypervisor".copy = lib.meta.getExe' self.packages.aarch64-linux.a653rs-linux-hypervisor "a653rs-linux-hypervisor";
              "/bin/hello_part".copy = lib.meta.getExe' self.packages.aarch64-linux.example-hello_part "hello_part";
              "/conf".copy = pkgs.writeText "conf" (builtins.readFile ./examples/hello_part/hello_part.yaml);
            };
          };
          a653rs-linux-hypervisor = naersk-lib.buildPackage
            rec {
              inherit env;
              pname = "a653rs-linux-hypervisor";
              root = ./.;
              cargoBuildOptions = x: x ++ [ "--package" pname ];
              cargoTestOptions = x: x ++ [ "--package" pname ];
            };
        } // (builtins.listToAttrs (builtins.map
          ({ name, partitions, ... }: {
            name = "example-${name}";
            value = naersk-lib.buildPackage
              {
                inherit env;
                pname = name;
                root = ./.;
                cargoBuildOptions = x: x ++ (cargoPackageList partitions);
                cargoTestOptions = x: x ++ (cargoPackageList partitions);
              };
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
                PATH="${fenix.latest.rustc}/bin:$PATH"
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
                PATH="${fenix.latest.rustc}/bin:$PATH"
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
              ({ name, partitions, preRun ? "" }: [
                {
                  name = "run-example-${name}";
                  command = ''
                    cd "$PRJ_ROOT"
                    # build partitions
                    cargo build --package ${concatStringsSep " " (cargoPackageList partitions)} --target ${rust-target} --release

                    # prepend PATH so that partition images can be found
                    PATH="target/${rust-target}/release:$PATH"

                    ${preRun}

                    # (build &) run hypervisor
                    RUST_LOG=''${RUST_LOG:=trace} cargo run --package a653rs-linux-hypervisor --release -- "examples/${name}/${name}.yaml" $@
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

