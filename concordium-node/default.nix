{ pkgs ? import <nixpkgs> { } }:

let
  moz_overlay = import (builtins.fetchTarball
  "https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
  rustStableChannel =
  (nixpkgs.rustChannelOf { channel = "stable"; }).rust.override {
    extensions =
    [ "rust-src" "rls-preview" "clippy-preview" "rustfmt-preview" ];
  };
in with pkgs;

let
  rustPlatform = pkgs.makeRustPlatform {
    cargo = rustStableChannel;
    rustc = rustStableChannel;
  };

in rustPlatform.buildRustPackage rec {
  name = "concordium-p2p-client-${version}";
  version = "0.1.35.3";
  src = ./.;
  RUST_BACKTRACE = 1;
  hardeningDisable = [ "all" ];
  cargoBuildFlags = [ "--features=static" ];
  buildInputs = with pkgs; [
    pkgconfig
    openssl
    cmake
    protobuf
    gmp
    numactl
    perl
    unbound
    gcc
  ];
  cargoSha256 = "1lay053m3vk6lzzm9iac6bmnic0qn9xsi9775hv31a1pcf5m7pa0";
  meta = with pkgs.stdenv.lib; {
    description = "Concordium AG";
    homepage = "https://www.concordium.com";
    license = licenses.mit;
  };
  doCheck = false;
}
