{
  description = "Hoshi";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      # Target system(s) — adjust if you need macOS or other
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
      {
        devShells = forAllSystems (system:
          let
            pkgs = nixpkgs.legacyPackages.${system};
            runtimeLibs = with pkgs; [
              # GTK4 / Adwaita
              gtk4
              libadwaita

              libx11
              libxcb
              libxkbcommon
              libxcursor
              libxi
              libxrandr
              libxinerama
              libxxf86vm

              # OpenGL loader / drivers
              mesa
              libglvnd
              glslang # or shaderc
              vulkan-headers
              vulkan-loader

              # Audio/fonts often needed by frameworks
              alsa-lib
              fontconfig
            ];
          in
            {
              default = pkgs.mkShell {
                strictDeps = true;
                nativeBuildInputs = with pkgs; [
                  pkg-config
                  rustc
                  cargo
                  cargo-nextest
                  cargo-watch
                  rustfmt
                  clippy
                  rust-analyzer
                  bacon

                  mold
                  clang

                  nodejs_24
                  typescript-language-server
                  biome

                  tokei
                ];

                buildInputs = runtimeLibs;
                LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
                GSETTINGS_SCHEMA_DIR = "${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}/glib-2.0/schemas";

                RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
                RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
                LD = "${pkgs.mold}/bin/mold";
                CC = "${pkgs.llvmPackages.clang}/bin/clang";
                CXX = "${pkgs.llvmPackages.clang}/bin/clang++";
              };
            });
      };
}
