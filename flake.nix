{
  description = "rust2: Rust to other languages and faster builds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # The quest CLI, which also serves the quest guide and skills the stubs in
    # .claude/skills call. Bump the rev to upgrade them.
    quest = {
      url = "github:moq-dev/quest/dc8008bc9e846a206403b1b738ede19fdd797f5e";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
    # The daemon the benchmark's ra-jit variant measures. Bumping the rev is a
    # benchmark corpus change: numbers before and after it do not compare.
    ra-jit = {
      url = "github:indexable-inc/ra-jit/d4f6027e82d312bf805ebfe6252369f6048a1e18";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
      inputs.flake-utils.follows = "flake-utils";
    };
    # The compiler cache the benchmark's Nix comparison measures, at the release
    # moq's CI runs. Bumping it is a corpus change, like ra-jit's.
    mbx = {
      url = "github:jdx/mr-boxington/v1.27.0";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      quest,
      ra-jit,
      mbx,
      ...
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          # The benchmark's nightly levers (--jobs-frontend, cranelift, self-profile).
          # Bumping the date is a benchmark corpus change: numbers before and
          # after it do not compare.
          nightly = pkgs.rust-bin.nightly."2026-10-22".minimal.override {
            extensions = [ "rustc-codegen-cranelift-preview" ];
          };
          # ra-jit works with one rustc only. This is the toolchain its flake builds
          # with, so the daemon and the variant's plain cargo share it.
          raJitToolchain = pkgs.rust-bin.stable."1.99.0".default.override {
            extensions = [ "rust-src" ];
          };
          # The TypeScript driver's own nightly, kept off PATH so the other projects keep
          # the repo toolchain. ts/justfile puts it first for its recipes.
          tsToolchain = pkgs.rust-bin.fromRustupToolchainFile ./ts/rust-toolchain.toml;
          quest-cli = quest.packages.${system}.default;
          # std checked once per toolchain for the TypeScript driver's target, with the
          # profile translation reads its MIR at, as the sysroot rust2-ts takes instead of
          # checking std with -Zbuild-std in every target directory. Offline, from the
          # crates rust-src vendors.
          rust2-ts-std = pkgs.stdenv.mkDerivation {
            name = "rust2-ts-std";
            dontUnpack = true;
            nativeBuildInputs = [ tsToolchain ];
            buildPhase = ''
              export CARGO_HOME=$TMPDIR/cargo CARGO_UNSTABLE_PROFILE_RUSTFLAGS=true
              mkdir -p sysroot/src && cd sysroot
              printf '[package]\nname = "sysroot"\nversion = "0.0.0"\nedition = "2024"\n' > Cargo.toml
              touch src/lib.rs
              # The crates ts/src/cli.rs's STD names.
              cargo check --offline --target wasm32-unknown-unknown -Zbuild-std=core,alloc,std,panic_abort \
                --config "source.crates-io.replace-with = 'vendored'" \
                --config "source.vendored.directory = '${tsToolchain}/lib/rustlib/src/rust/library/vendor'" \
                --config ${./ts/src/profile.toml}
            '';
            installPhase = ''
              lib=$out/lib/rustlib/wasm32-unknown-unknown/lib
              mkdir -p $lib
              find target/wasm32-unknown-unknown -name 'lib*.rmeta' ! -name 'libsysroot-*' -exec cp -t $lib {} +
            '';
          };
          # The driver links its nightly's rustc libraries, and runs cargo from it too.
          rust2-ts =
            (pkgs.makeRustPlatform {
              cargo = tsToolchain;
              rustc = tsToolchain;
            }).buildRustPackage
              {
                pname = "rust2-ts";
                version = "0.0.0";
                # With the driver front end it shares, which it depends on by path.
                src = pkgs.lib.fileset.toSource {
                  root = ./.;
                  fileset = pkgs.lib.fileset.unions [
                    ./ts
                    ./driver
                  ];
                };
                buildAndTestSubdir = "ts";
                cargoRoot = "ts";
                cargoLock.lockFile = ./ts/Cargo.lock;
                cargoBuildFlags = [
                  "--package"
                  "rust2-ts"
                ];
                RUSTFLAGS = "-C link-args=-Wl,-rpath,${tsToolchain}/lib";
                nativeBuildInputs = [ pkgs.makeWrapper ];
                # tsc checks bindings, from the caller's PATH first.
                postInstall = ''
                  wrapProgram $out/bin/rust2-ts --set-default CARGO ${tsToolchain}/bin/cargo \
                    --set-default RUST2_TS_SYSROOT ${rust2-ts-std} \
                    --suffix PATH : ${pkgs.typescript}/bin
                '';
                # The tests need node, tsc, and wasm builds: `just ts test` runs them.
                doCheck = false;
              };
          # The mono budget lint, on the same nightly for the same reasons.
          rust2-lint =
            (pkgs.makeRustPlatform {
              cargo = tsToolchain;
              rustc = tsToolchain;
            }).buildRustPackage
              {
                pname = "rust2-lint";
                version = "0.0.0";
                src = pkgs.lib.fileset.toSource {
                  root = ./driver;
                  fileset = pkgs.lib.fileset.unions [
                    ./driver/Cargo.toml
                    ./driver/Cargo.lock
                    ./driver/src
                  ];
                };
                cargoLock.lockFile = ./driver/Cargo.lock;
                cargoBuildFlags = [
                  "--bin"
                  "rust2-lint"
                ];
                RUSTFLAGS = "-C link-args=-Wl,-rpath,${tsToolchain}/lib";
                nativeBuildInputs = [ pkgs.makeWrapper ];
                postInstall = ''
                  wrapProgram $out/bin/rust2-lint --set-default CARGO ${tsToolchain}/bin/cargo
                '';
                # `just driver test` runs the fixtures.
                doCheck = false;
              };

          # Every generator binary of the UniFFI fork, built once from its lockfile.
          uniffi =
            (pkgs.makeRustPlatform {
              cargo = toolchain;
              rustc = toolchain;
            }).buildRustPackage
              {
                pname = "rust2-uniffi";
                version = (builtins.fromTOML (builtins.readFile ./uniffi/uniffi/Cargo.toml)).package.version;
                # The root workspace, without the other projects.
                src = pkgs.lib.fileset.toSource {
                  root = ./.;
                  fileset = pkgs.lib.fileset.unions [
                    ./Cargo.toml
                    ./Cargo.lock
                    ./uniffi
                  ];
                };
                cargoLock.lockFile = ./Cargo.lock;
                cargoBuildFlags = [
                  "--package=uniffi"
                  "--package=uniffi-bindgen-go"
                  "--package=uniffi-dart"
                  "--package=uniffi-bindgen-cpp"
                  "--bins"
                ];
                buildFeatures = [
                  "uniffi/cli"
                  "uniffi-dart/binary"
                ];
                # The fixture suite needs every foreign toolchain; `just uniffi fixtures` runs it.
                doCheck = false;
              };

          # One generator's binaries, so a consumer can put only that one on PATH.
          generator =
            name: bins:
            pkgs.runCommand name { } ''
              mkdir -p $out/bin
              ${pkgs.lib.concatMapStringsSep "\n" (bin: "ln -s ${uniffi}/bin/${bin} $out/bin/") bins}
            '';

          # Kotlin's generated async code, UniFFI's and rust2-ffi's, needs coroutines on the
          # classpath; nixpkgs has no jar.
          kotlinx-coroutines = pkgs.fetchurl {
            url = "https://repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.10.2/kotlinx-coroutines-core-jvm-1.10.2.jar";
            hash = "sha256-XKF1s43zMf1kFVs1zYyuElH6nuNpcJs21C4KKIzM4/0=";
          };
        in
        {
          packages = {
            quest = quest-cli;
            ts = rust2-ts;
            lint = rust2-lint;
            inherit uniffi;
            uniffi-bindgen = generator "uniffi-bindgen" [
              "uniffi-bindgen"
              "uniffi-bindgen-swift"
            ];
            uniffi-bindgen-go = generator "uniffi-bindgen-go" [ "uniffi-bindgen-go" ];
            uniffi-bindgen-dart = generator "uniffi-bindgen-dart" [ "uniffi_bindgen_dart" ];
            uniffi-bindgen-cpp = generator "uniffi-bindgen-cpp" [ "uniffi-bindgen-cpp" ];
          }
          # ra-jit and its toolchain for the benchmark's ra-jit variants, which build it
          # only when they run: it compiles from source. ra-jit skips Intel macOS.
          // pkgs.lib.optionalAttrs (ra-jit.packages ? ${system}) {
            bench-ra-jit = pkgs.symlinkJoin {
              name = "bench-ra-jit";
              paths = [
                ra-jit.packages.${system}.ra-jit
                raJitToolchain
              ];
            };
          }
          # What `just bench-nix` runs beyond the build-bench shell, built only then:
          # mbx compiles from source.
          // pkgs.lib.optionalAttrs (mbx.packages ? ${system}) {
            bench-nix = pkgs.symlinkJoin {
              name = "bench-nix";
              paths = [
                mbx.packages.${system}.mbx
                pkgs.crate2nix
              ];
            };
          };
          # `just bench` enters this shell; the nightly toolchain stays off PATH
          # so stable remains the default. It holds only what the benchmark runs:
          # a PR runs bench.yml only when this shell, or another output it lists
          # (bench-ra-jit), changes derivation.
          devShells.build-bench = pkgs.mkShell {
            packages =
              with pkgs;
              [
                toolchain
                just
                jq
                git
                taplo
                hyperfine
                cargo-nextest
                measureme
              ]
              # The alternative linkers the stable variants measure are ELF-only.
              ++ lib.optionals stdenv.hostPlatform.isLinux [
                mold
                wild
              ];
            RUST2_BENCH_NIGHTLY = "${nightly}/bin";
          };
          devShells.default = pkgs.mkShell {
            packages = with pkgs; [
              toolchain
              quest-cli
              just
              jq
              git
              gh
              cargo-nextest
              cargo-deny
              cargo-shear
              cargo-sort
              actionlint
              shellcheck
              shfmt
              taplo
              # bench/ffi/run.sh serializes runs that share a work directory.
              flock
              nixfmt
              # The TypeScript driver's tests run and type-check its output, and bundle a
              # caller with Bun and esbuild to check that unused output shakes out.
              nodejs
              typescript
              bun
              esbuild
              # rust2-ffi's tests compile its generated Kotlin and run it on the JVM.
              kotlin
            ];
            # ... and its generated Swift, where nixpkgs builds Swift: not on aarch64-linux.
            nativeBuildInputs = pkgs.lib.optional (!pkgs.swift.meta.broken) pkgs.swift;
            RUST2_TS_TOOLCHAIN = tsToolchain;
            RUST2_TS_SYSROOT = rust2-ts-std;
            KOTLINX_COROUTINES = "${kotlinx-coroutines}";
          };
          # `just ts bench` adds Chrome and Firefox as engines. Apart for their size.
          devShells.bench = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.chromium
              pkgs.firefox
            ];
            # `just ffi bench` builds moq's rs/moq-c, whose VAAPI bindings run bindgen.
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          });
          # Clang beside the default shell's GCC, which the nightly builds the exporter's C++ with.
          devShells.clang = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clang ];
          });
          # `just ffi android` builds the Kotlin's library for an Android device, with the NDK
          # cargo-ndk finds through ANDROID_NDK_HOME: the SDK and NDK are the caller's, as moq's
          # Android build takes the GitHub runner image's.
          devShells.android = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.cargo-ndk ];
            RUST2_TS_TOOLCHAIN = tsToolchain.override {
              targets = (builtins.fromTOML (builtins.readFile ./ts/rust-toolchain.toml)).toolchain.targets ++ [
                "aarch64-linux-android"
                "x86_64-linux-android"
              ];
            };
          });
          # Every foreign toolchain the UniFFI fixture suite runs, apart for their size.
          devShells.uniffi = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.python3
              pkgs.kotlin
              pkgs.swift
              pkgs.go
              pkgs.dart
              pkgs.cmake
            ];
            CLASSPATH = "${pkgs.jna}/share/java/jna.jar:${kotlinx-coroutines}";
          });
          formatter = pkgs.nixfmt;
          checks.quest-tree = pkgs.runCommand "quest-tree" { nativeBuildInputs = [ quest-cli ]; } ''
            quest --root ${self} check
            touch "$out"
          '';
        }
      );
}
