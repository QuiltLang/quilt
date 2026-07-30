{
  description = "Quilt — polyglot metaprogramming language";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # python3 + pytest so `bin/test-py` can `python3 -m pytest`; the PyO3
        # cdylib's libpython comes from the same interpreter (see LD_LIBRARY_PATH).
        python = pkgs.python3.withPackages (ps: [ ps.pytest ]);
      in
      {
        devShells.default = pkgs.mkShellNoCC {
          packages = [
            # `cargo insta review` — the interactive accept/reject pass over
            # changed expander snapshots (issue #157). Tests themselves need
            # only the `insta` crate; this is the review UI.
            pkgs.cargo-insta
            pkgs.cargo-nextest
            pkgs.lolcat
            # maturin builds the quilt_python PyO3 module (`bin/build-py`).
            pkgs.maturin
            pkgs.nodejs
            # quilt_python (PyO3 cdylib) links libpython; provide python3 — plus
            # pytest for bin/test-py — from the flake so the env is self-contained
            # instead of relying on a python3 from the ambient/global profile.
            python
            pkgs.rust-script
            pkgs.rustup
            pkgs.tree-sitter
            # wasm-pack builds the quilt-wasm runtime (`bin/build-ts`), which
            # `quilt run` needs to run a .ts.quilt file at all.
            #
            # This used to be left out on the grounds that it "pulls its own
            # toolchain", with CI curl|sh-ing the official installer instead.
            # That installer cannot work here: it copies the binary next to the
            # `cargo` it finds on PATH — under this shell, the read-only
            # /nix/store rustup — and ignores $CARGO_HOME, so it fails with
            # EACCES. So nobody in the devShell could build wasm, and the
            # nightly runtime-parity job silently skipped its wasm third for
            # want of a binary it never actually installed.
            pkgs.wasm-pack
          ];

          RUST_BACKTRACE = "1";

          # quilt_python (PyO3 cdylib) links libpython, so the cargo-test binary
          # loads libpython3.*.so at run time. Point the dynamic loader at the
          # flake's python (Linux/CI; ignored by dyld on macOS) so the env is
          # self-contained — this replaces CI's old setup-python-derived
          # LD_LIBRARY_PATH step.
          LD_LIBRARY_PATH = "${pkgs.python3}/lib";
        };
      }
    );
}
