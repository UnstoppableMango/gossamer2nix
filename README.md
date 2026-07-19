# gossamer2nix

Nix builder for [Gossamer](https://github.com/danpozmanter/gossamer).
Nix-native dependency conversion + reproducible builds, in the spirit of [gomod2nix](https://github.com/nix-community/gomod2nix).

## Status

Early — no implementation yet.

See [GOALS.md](./GOALS.md) for scope.

## Background

Gossamer is a Rust-flavored, Go-shaped runtime language (source files use the `.gos` extension).
Its `gos` toolchain has its own package manager (`gos add/remove/tidy/vendor/publish`, backed by an Ed25519-signed registry) and projects declare dependencies in a `project.toml` manifest.
There's currently no way to build Gossamer projects reproducibly under Nix.

## How

- Parse `project.toml` (+ its lockfile) and generate a Nix-consumable deps lock file capturing pinned versions + content hashes, via a `gossamer2nix` CLI (mirrors `gomod2nix generate`).
- `buildGossamerApplication` takes a source tree + the generated lock and produces a derivation that invokes `gos build`; supports standard `overrideAttrs`/`override`.
- Deps are fetched via Nix fixed-output derivations ahead of build time, so `nix build` itself needs no network access.

## Prior art

- [gomod2nix](https://github.com/nix-community/gomod2nix)
- `buildGoModule` / `buildGoApplication` (nixpkgs)
- [npmlock2nix](https://github.com/nix-community/npmlock2nix)
- [crane](https://github.com/ipetkov/crane) (Rust)
