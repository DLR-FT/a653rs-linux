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
    utils.lib.eachSystem [ "x86_64-linux" ] (system:
      let
        lib = nixpkgs.lib;
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ devshell.overlay ];
        };

        rust-toolchain = with fenix.packages.${system};
          combine [
            stable.rustc
            stable.cargo
            stable.clippy
            latest.rustfmt
            targets.x86_64-unknown-linux-musl.stable.rust-std
          ];

        # overrides a naersk-lib which uses the stable toolchain expressed above
        naersk-lib = (naersk.lib.${system}.override {
          cargo = rust-toolchain;
          rustc = rust-toolchain;
        });
      in
      rec {
        packages = {
          linux-apex-hypervisor = naersk-lib.buildPackage rec {
            pname = "linux-apex-hypervisor";
            root = ./.;
            cargoBuildOptions = x: x ++ [ "--package" pname ];
            cargoTestOptions = x: x ++ [ "--package" pname ];
          };
          hello-part = naersk-lib.buildPackage rec {
            pname = "hello_part";
            root = ./.;
            cargoBuildOptions = x:
              x ++ [ "--package" pname "--target" "x86_64-unknown-linux-musl" ];
            cargoTestOptions = x:
              x ++ [ "--package" pname "--target" "x86_64-unknown-linux-musl" ];
          };
        };

        defaultPackage = packages.linux-apex-hypervisor;

        devShells.default = (pkgs.devshell.mkShell {
          imports = [ "${devshell}/extra/git/hooks.nix" ];
          name = "linux-apex-dev-shell";
          packages = with pkgs; [
            clang
            rust-toolchain
            rust-analyzer
            cargo-outdated
            cargo-udeps
            cargo-watch
            nixpkgs-fmt
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
                PATH=${fenix.packages.${system}.latest.rustc}/bin:$PATH
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
              name = "clippy-watch-hello-example";
              command = ''
                cargo watch -x "clippy -p hello_part --target x86_64-unknown-linux-musl"
              '';
              help = "Continuesly clippy \"hello\" example";
              category = "dev";
            }
            {
              name = "run-hypervisor-hello-example-scoped";
              command = "systemd-run --user --scope run-hypervisor-hello-example";
              help = "Run Hypervisor with the \"hello\" example with systemd-run";
              category = "dev";
            }
            {
              name = "run-hypervisor-hello-example";
              command = ''
                cd $PRJ_ROOT
                nix build .#hello-part
                RUST_LOG=''${RUST_LOG:=trace} \
                  nix run .#linux-apex-hypervisor \
                  -- examples/hello_part/hypervisor_config.yaml
              '';
              help = "Run Hypervisor with the \"hello\" example";
              category = "dev";
            }
            {
              name = "verify-no_std";
              command = ''
                cd $PRJ_ROOT
                cargo build --target thumbv6m-none-eabi --no-default-features
              '';
              help =
                "Verify that the library builds for no_std without std-features";
              category = "dev";
            }
          ];
        });
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
      });
}

