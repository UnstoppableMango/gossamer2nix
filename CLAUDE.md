# gossamer2nix

Nix builder for [Gossamer](https://github.com/danpozmanter/gossamer) (Rust-flavored, Go-shaped lang, `.gos` files). In the spirit of `gomod2nix`. See [GOALS.md](./GOALS.md) for scope/non-goals.

**Status: early — no `gossamer2nix` CLI / deps-lock generation implemented yet.** Only the Nix builder side exists so far.

## Commands

```bash
nix build .#          # build (Makefile: make build)
nix flake check       # lint (Makefile: make check / make lint)
nix fmt               # format via treefmt, nixfmt (Makefile: make format / make fmt)
nix flake update       # update flake inputs (Makefile: make update)
```

`direnv` (`.envrc` → `use flake`) drops you into `devShells.default` (gnumake, gossamer, nixfmt).

## Architecture

- `flake.nix` — flake-parts based. Pulls in `mangopkgs` (github:unmango/pkgs) overlay for the `gossamer` package itself — nixpkgs upstream has no `gossamer` derivation yet.
- `nix/default.nix` — exports `buildGossamerApplication` (via `pkgs.callPackage ./builder.nix { }`).
- `nix/builder.nix` — the actual builder: `stdenv.mkDerivation` wrapper running `gos build --release --out-dir dist`, installs everything from `dist/*` to `$out/bin`. Takes `gosBuildFlags` (extra args to `gos build`, e.g. `--target`, `--locked`). Accepts arbitrary passthrough attrs like `buildGoModule`-style builders.
- `nix/checks.nix` — flake checks; currently just `hello-app`, a smoke build via `gos new` scaffolding + `buildGossamerApplication`.

## Gotchas

- Binary cache: `mangopkgs.cachix.org` configured in `flake.nix` `nixConfig` — needed to avoid rebuilding the `gossamer` toolchain itself from the overlay.
- No deps-lock generation yet, so `buildGossamerApplication` currently has no offline/sandboxed dependency fetching — that's the main planned feature (see GOALS.md), not yet real.
- x86_64-darwin dropped from supported systems (see commit history) — don't assume it works.
