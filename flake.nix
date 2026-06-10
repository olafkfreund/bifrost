{
  description = "Bifrost — orchestration + intelligence layer for ADO → GitHub Actions migration";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , rust-overlay
    , flake-utils
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Honour the exact toolchain pinned in rust-toolchain.toml (channel + clippy/rustfmt).
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        devShells.default = pkgs.mkShell {
          name = "bifrost";

          packages = with pkgs; [
            # Rust control plane (reqwest uses rustls, so no OpenSSL/pkg-config needed)
            rustToolchain
            cargo-watch
            cargo-nextest

            # Portal
            nodejs_22

            # Migration tooling: the official Importer is a gh extension + Docker
            # image (not in nixpkgs); these provide the host side.
            gh
            azure-cli
            docker-client

            # Nix authoring
            nixd
            nixpkgs-fmt
            statix
            deadnix

            # Misc
            git
            jq
          ];

          # rustls everywhere — keep the shell free of native TLS/OpenSSL deps.
          shellHook = ''
            echo "🌈  Bifrost dev shell"
            echo "    rust : $(rustc --version 2>/dev/null)"
            echo "    node : $(node --version 2>/dev/null)"
            echo "    gh   : $(gh --version 2>/dev/null | head -1)"
            echo ""
            echo "    Secrets: tokens live in .envrc (gitignored). Run: source .envrc"
            if ! gh extension list 2>/dev/null | grep -q actions-importer; then
              echo ""
              echo "    ⚠️  gh-actions-importer not installed. Install it once (writable \$HOME):"
              echo "        gh extension install github/gh-actions-importer"
              echo "        gh actions-importer configure   # uses GITHUB_TOKEN + AZDO_PAT"
            fi
          '';
        };

        # `nix fmt` formats the flake.
        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
