# A reproducible development shell, without flakes:  nix-shell
#
# nixpkgs is pinned by commit and hash, so it needs no channel and gives everyone the same
# toolchain. CI pins its own Rust (rust-toolchain.toml, the minimum supported version); this shell
# uses the pinned nixpkgs' current Rust, which is newer, so it is for developing, not for judging
# whether the minimum still builds. The packaging tools (cargo-deb, cargo-generate-rpm) are not
# here: only the release workflow needs them, and it installs them itself.
let
  pkgs = import (fetchTarball {
    # nixos-26.05
    url = "https://github.com/NixOS/nixpkgs/archive/c56a87d8e41ce69bd2af57eb35dcc117a9dfca33.tar.gz";
    sha256 = "06g8y8y9bysnpxrl6bfqmn63l8w2lx2yhj0618r34j5jfqy5vjpx";
  }) { };
in
pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    clippy
    rustfmt
    rust-analyzer
    git
    jq
  ];
}
