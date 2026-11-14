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
      url = "github:moq-dev/quest";
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
    # CodSpeed's instrument-hooks, which the binding benchmark's programs call around each
    # case its CPU simulation counts.
    instrument-hooks = {
      url = "github:CodSpeedHQ/instrument-hooks/8fd5b878499f819561b33219cdafaa6f8c28b4e4";
      flake = false;
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
      instrument-hooks,
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
          # ra-jit's dist build finds libzstd through pkg-config, which its flake never declares,
          # so it built only where nixpkgs put pkg-config on the build's PATH. Declare both here.
          # nixpkgs also points tikv-jemalloc-sys at a jemalloc path that does not exist, which
          # the build refuses. Unset it, so jemalloc builds from source as it did before.
          # Then wrap the binaries as ra-jit's `daemon` does, over the toolchain above.
          raJitDist = ra-jit.packages.${system}.dist.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.pkg-config ];
            buildInputs = (old.buildInputs or [ ]) ++ [ pkgs.zstd ];
            preBuild = ''
              unset JEMALLOC_OVERRIDE
            ''
            + (old.preBuild or "");
          });
          raJitDaemon =
            pkgs.runCommand "ra-jit-${raJitDist.version}"
              {
                nativeBuildInputs = [ pkgs.makeWrapper ];
              }
              ''
                mkdir -p $out/bin
                for bin in ${raJitDist}/bin/*; do
                  makeWrapper $bin $out/bin/$(basename $bin) --prefix PATH : ${raJitToolchain}/bin
                done
              '';
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
          # rust2-ffi's generated async Kotlin needs coroutines on the
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
          }
          # ra-jit and its toolchain for the benchmark's ra-jit variants, which build it
          # only when they run: it compiles from source. ra-jit skips Intel macOS.
          // pkgs.lib.optionalAttrs (ra-jit.packages ? ${system}) {
            bench-ra-jit = pkgs.symlinkJoin {
              name = "bench-ra-jit";
              paths = [
                raJitDaemon
                raJitToolchain
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
              nixfmt
              # The TypeScript driver's tests run and type-check its output, and bundle a
              # caller with Bun and esbuild to check that unused output shakes out.
              nodejs
              typescript
              bun
              esbuild
              # Its wasm-bindgen fixture's native side runs through the CLI's JS glue, whose
              # version must match the fixture's locked wasm-bindgen exactly.
              wasm-bindgen-cli_0_2_129
              # rust2-ffi's tests compile its generated Kotlin and run it on the JVM, run and
              # type-check its generated Python, run its Dart, and build its Go through cgo.
              kotlin
              python3
              mypy
              pyright
              # rust2-go's tests build and run its output against native Rust.
              go
              dart
            ];
            # ... and its generated Swift, unless nixpkgs marks the compiler broken on this system.
            # The `swift` wrapper doesn't copy its compiler's `broken`, so ask the compiler.
            nativeBuildInputs = pkgs.lib.optional (!pkgs.swift.swiftc.meta.broken) pkgs.swift;
            RUST2_TS_TOOLCHAIN = tsToolchain;
            RUST2_TS_SYSROOT = rust2-ts-std;
            KOTLINX_COROUTINES = "${kotlinx-coroutines}";
            # The pinned toolchain goes on PATH ahead of everything the host had, which
            # shadows the Cargo shim `mbx setup` installs. Put it back in front, as moq's dev
            # shell does, so cargo builds through mbx's cache wherever the host set one up,
            # the land gate included. The nightly projects' justfiles keep it ahead of their
            # toolchain through RUST2_CARGO_SHIM. `setup --status` names the shim on its first
            # line even when it is missing, and its exit code also fails over unrelated setup,
            # so only the shim existing decides.
            shellHook = ''
              # An outer shell's shim must not outlive a host without one: the justfiles
              # would still put its directory ahead of the toolchain.
              unset RUST2_CARGO_SHIM
              shim=$(mbx setup --status 2>/dev/null | sed -n '1s/.*: //p' || true)
              if [ -x "$shim" ]; then
                export RUST2_CARGO_SHIM=''${shim%/*}
                export PATH="$RUST2_CARGO_SHIM:$PATH"
              fi
              unset shim
            '';
          };
          # `just ts bench` adds Chrome and Firefox as engines. Apart for their size.
          devShells.bench = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [
              pkgs.chromium
              pkgs.firefox
            ];
            # `just ffi bench` builds moq's rs/moq-c, whose VAAPI bindings run bindgen.
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            # ... and its programs over instrument-hooks, for CodSpeed.
            CODSPEED_INSTRUMENT_HOOKS = "${instrument-hooks}";
          });
          # Clang beside the default shell's GCC, which the nightly builds the exporter's C++ with.
          devShells.clang = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clang ];
          });
          # `just ffi sanitize` builds the glue with the nightly's sanitizers, linked by clang, and
          # runs it under the nightly's Miri. llvm-symbolizer names the frames of a report.
          devShells.sanitize = self.devShells.${system}.clang.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.llvm ];
            RUST2_TS_TOOLCHAIN = tsToolchain.override {
              extensions =
                (builtins.fromTOML (builtins.readFile ./ts/rust-toolchain.toml)).toolchain.components
                ++ [
                  "miri"
                ];
            };
          });
          # `just ffi android` builds the Kotlin's library for an Android device, with the NDK
          # cargo-ndk finds through ANDROID_NDK_HOME, and `just ffi moq-api` stages moq's API on
          # each Android ABI moq ships: the SDK and NDK are the caller's, as moq's Android build
          # takes the GitHub runner image's.
          devShells.android = self.devShells.${system}.default.overrideAttrs (old: {
            nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.cargo-ndk ];
            RUST2_TS_TOOLCHAIN = tsToolchain.override {
              targets = (builtins.fromTOML (builtins.readFile ./ts/rust-toolchain.toml)).toolchain.targets ++ [
                "aarch64-linux-android"
                "armv7-linux-androideabi"
                "x86_64-linux-android"
              ];
            };
          });
          formatter = pkgs.nixfmt;
          checks.quest-tree = pkgs.runCommand "quest-tree" { nativeBuildInputs = [ quest-cli ]; } ''
            quest --root ${self} check
            touch "$out"
          '';
        }
      );
}
