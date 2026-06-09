{
  description = "Quilt — multi-language metaprogramming system";

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
            pkgs.rust-script
            pkgs.rustup
            pkgs.tree-sitter
          ];

          RUST_BACKTRACE = "1";
        };
      }
    );
}
