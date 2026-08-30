{
  description = "HyprOsk: Native Wayland On-Screen Keyboard for Hyprland with HeliBoard typing experience";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "hyprosk";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            wayland-scanner
          ];

          buildInputs = with pkgs; [
            wayland
            wayland-protocols
            libxkbcommon
          ];

          meta = with pkgs.lib; {
            description = "Fast, lightweight native Wayland On-Screen Keyboard designed for Hyprland";
            homepage = "https://github.com/Ziggs25/HyprOsk";
            license = with licenses; [ mit asl20 ];
            maintainers = [ ];
            platforms = platforms.linux;
            mainProgram = "hyprosk";
          };
        };

        packages.hyprosk = self.packages.${system}.default;

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            rustc
            cargo
            rustfmt
            clippy
            wayland-scanner
          ];

          buildInputs = with pkgs; [
            wayland
            wayland-protocols
            libxkbcommon
          ];

          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath (with pkgs; [
            wayland
            libxkbcommon
          ])}";
        };
      }
    ) // {
      # NixOS Module (system-level or user service)
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.hyprosk;
        in
        {
          options.programs.hyprosk = {
            enable = lib.mkEnableOption "HyprOsk Wayland on-screen keyboard";
            package = lib.mkPackageOption self.packages.${pkgs.system} "hyprosk" { };
            autoStart = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Automatically start HyprOsk as a systemd user service inside Wayland/Hyprland sessions.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];

            systemd.user.services.hyprosk = lib.mkIf cfg.autoStart {
              description = "HyprOsk On-Screen Keyboard Daemon";
              wantedBy = [ "graphical-session.target" ];
              partOf = [ "graphical-session.target" ];
              after = [ "graphical-session.target" ];
              serviceConfig = {
                ExecStart = "${cfg.package}/bin/hyprosk daemon";
                Restart = "on-failure";
                RestartSec = 1;
              };
            };
          };
        };

      nixosModules.hyprosk = self.nixosModules.default;

      # Home Manager Module
      homeManagerModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.programs.hyprosk;
        in
        {
          options.programs.hyprosk = {
            enable = lib.mkEnableOption "HyprOsk on-screen keyboard";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              description = "HyprOsk package to use.";
            };
            settings = lib.mkOption {
              type = lib.types.attrsOf lib.types.anything;
              default = {};
              description = "Configuration settings written to ~/.config/hyprosk/config.toml";
            };
            systemdTarget = lib.mkOption {
              type = lib.types.str;
              default = "hyprland-session.target";
              description = "Systemd graphical session target to bind the user service to.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];

            xdg.configFile."hyprosk/config.toml" = lib.mkIf (cfg.settings != {}) {
              source = (pkgs.formats.toml {}).generate "hyprosk-config.toml" cfg.settings;
            };

            systemd.user.services.hyprosk = {
              Unit = {
                Description = "HyprOsk On-Screen Keyboard Daemon";
                PartOf = [ cfg.systemdTarget ];
                After = [ cfg.systemdTarget ];
              };
              Service = {
                ExecStart = "${cfg.package}/bin/hyprosk daemon";
                Restart = "on-failure";
                RestartSec = "1s";
              };
              Install = {
                WantedBy = [ cfg.systemdTarget ];
              };
            };
          };
        };

      homeManagerModules.hyprosk = self.homeManagerModules.default;
    };
}
