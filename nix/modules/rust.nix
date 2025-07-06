{ inputs, ... }: {
  imports = [
    inputs.rust-flake.flakeModules.default
    inputs.rust-flake.flakeModules.nixpkgs
  ];
  perSystem =
    { config
    , self'
    , pkgs
    , lib
    , ...
    }: {
      rust-project.crates."pattern-main".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isDarwin (
          with pkgs.darwin.apple_sdk.frameworks; [
            IOKit
          ]
        );
      };

      # Define workspace members
      rust-project.crates."pattern-core".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isDarwin (
          with pkgs.darwin.apple_sdk.frameworks; [
            IOKit
          ]
        );
      };

      rust-project.crates."pattern-nd".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isDarwin (
          with pkgs.darwin.apple_sdk.frameworks; [
            IOKit
          ]
        );
      };

      rust-project.crates."pattern-mcp".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isDarwin (
          with pkgs.darwin.apple_sdk.frameworks; [
            IOKit
          ]
        );
      };

      rust-project.crates."pattern-discord".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isDarwin (
          with pkgs.darwin.apple_sdk.frameworks; [
            IOKit
          ]
        );
      };

      packages.default = self'.packages.pattern-main;
      packages.pattern = self'.packages.pattern-main;
    };
}
