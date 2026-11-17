{
  description = "Christmas snowfall overlay for Hyprland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "christmas-snow";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
            buildInputs = [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.vulkan-loader
            ];
            postFixup = ''
              wrapProgram "$out/bin/christmas-snow" \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
                  pkgs.vulkan-loader
                  pkgs.libGL
                ]}
            '';
            meta = {
              description = "A native Christmas snowfall overlay for Hyprland";
              homepage = "https://github.com/clemensrh/hyprsnow";
              license = pkgs.lib.licenses.mit;
              platforms = systems;
              mainProgram = "christmas-snow";
            };
          };
        });

      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.christmas-snow;
          package = self.packages.${pkgs.system}.default;
        in
        {
          options.programs.christmas-snow.enable =
            lib.mkEnableOption "the Christmas Snow Wayland overlay";
          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ package ];
            systemd.user.services.christmas-snow = {
              description = "Christmas Snow Wayland overlay";
              wantedBy = [ "graphical-session.target" ];
              after = [ "graphical-session.target" ];
              serviceConfig = {
                ExecStart = "${package}/bin/christmas-snow run";
                Restart = "on-failure";
                RestartSec = 2;
              };
            };
            systemd.user.services.christmas-snow-tray = {
              description = "Christmas Snow system tray";
              wantedBy = [ "graphical-session.target" ];
              after = [ "graphical-session.target" ];
              partOf = [ "graphical-session.target" ];
              serviceConfig = {
                ExecStart = "${package}/bin/christmas-snow tray";
                Restart = "on-failure";
                RestartSec = 2;
              };
            };
          };
        };

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          runtimeLibs = with pkgs; [
            libxkbcommon
            wayland
            vulkan-loader
          ];
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              pkg-config
              rustc
              rustfmt
            ];
            buildInputs = runtimeLibs;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          };
        });
    };
}
