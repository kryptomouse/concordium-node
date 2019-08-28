{ pkgs ? import <nixpkgs> { } }:

let
  moz_overlay = import (builtins.fetchTarball
  "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
  rustStableChannel =
  (nixpkgs.rustChannelOf { channel = "1.37.0"; }).rust.override {
    extensions =
    [ "rust-src" "rls-preview" "clippy-preview" "rustfmt-preview" ];
  };
in with nixpkgs;
stdenv.mkDerivation {
  name = "concordium_shell";
  hardeningDisable = [ "all" ];
  buildInputs = [
    rustStableChannel
    protobuf
    pkgconfig
    unbound
    numactl
    gmp
    cmake
    curl
    gnutar
    capnproto
  ];
  shellHook = ''
    scripts/download-static-libs.sh
  '';
}
