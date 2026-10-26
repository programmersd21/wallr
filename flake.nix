{
  description = "GPU-accelerated wallpaper engine for Wayland";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default;
        llvmPackages = pkgs.llvmPackages;
      in
      {
        packages = {
          wallr = pkgs.rustPlatform.buildRustPackage {
            pname = "wallr";
            version = "0.8.4";
            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              rustToolchain
              pkgs.pkg-config
              llvmPackages.libclang
            ];

            buildInputs = [
              pkgs.wayland
              pkgs.wayland-protocols
              pkgs.ffmpeg
            ];

            # Skip tests that require a Wayland compositor
            doCheck = false;

            LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

            meta = with pkgs.lib; {
              description = "GPU-accelerated wallpaper engine for Wayland";
              homepage = "https://github.com/programmersd21/wallr";
              license = licenses.mit;
              maintainers = [ ];
              platforms = platforms.linux;
            };
          };

          default = self.packages.${system}.wallr;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.wallr ];

          packages = [
            rustToolchain
            pkgs.rust-analyzer
            pkgs.clippy
            pkgs.rustfmt
            llvmPackages.libclang
            pkgs.nil
            pkgs.nixd
          ];

          LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };
      });
}
