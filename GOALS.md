# Goals

- Convert Gossamer project dependencies into a Nix-consumable form.
- Provide a CLI to generate/update that dependency data.
- Expose a `buildGossamerApplication` Nix builder for Gossamer projects.
- Fully reproducible, offline/sandboxed builds.
- Nixpkgs-idiomatic: overlay-friendly, works with flakes and legacy `default.nix`.
- Support `[rust-bindings]` (FFI) deps by building the underlying Cargo crate with existing Rust-on-Nix tooling (e.g. crane, naersk, `buildRustPackage`), not a bespoke Cargo resolver.

## Non-goals

- Not a general Rust/Go lang2nix tool.
- Not replacing `gos`'s own registry/vendor workflow; only bridging it to Nix.
