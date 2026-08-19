{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    rustc
    cargo
    rustfmt
    clippy
  ];

  buildInputs = with pkgs; [
    wayland
    wayland-protocols
    wayland-scanner
    libxkbcommon
  ];

  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath (with pkgs; [
    wayland
    libxkbcommon
  ])}";
}
