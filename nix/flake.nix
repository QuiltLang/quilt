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
      in
      {
        devShells.default = pkgs.mkShellNoCC {
          packages = [
            pkgs.cargo-nextest
            pkgs.lolcat
            pkgs.nodejs
            # quilt_python (PyO3 cdylib) links libpython; provide it from the
            # flake so the env is self-contained instead of relying on a
            # python3 from the ambient/global profile.
            pkgs.python3
            pkgs.rust-script
            pkgs.rustup
            pkgs.tree-sitter
          ];

          RUST_BACKTRACE = "1";
        };
      }
    );
}
