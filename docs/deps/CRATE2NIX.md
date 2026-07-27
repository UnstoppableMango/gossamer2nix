# `crate2nix`: Bridging Cargo into Nix

Read 2026-07-26 to ground this repo's eventual handling of Gossamer's `[rust-bindings]` FFI dependency graph (`../DEPS.md` §3.1) in how the most established Cargo/Nix tool actually works. Sources: `crate2nix/src/{lock,resolve,prefetch,metadata}.rs`, `crate2nix/templates/Cargo.nix.tera`, `crate2nix/templates/nix/crate2nix/default.nix`, `nixpkgs/pkgs/build-support/rust/{fetchcrate.nix,build-rust-crate/*}`.

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/crate2nix](https://github.com/nix-community/crate2nix) (`master`) |
| **Ecosystem** | Rust / Cargo |
| **Maintenance status** | Maintained but slow-moving — 0.15.0 shipped after ~2 years; maintainer publicly seeking a co-maintainer |
| **Ecosystem input** | `Cargo.lock` (`source`, `checksum` fields) via `cargo metadata` — not hand-parsed |
| **Generated output** | `Cargo.nix` (one attrset per resolved crate) + `crate-hashes.json` sidecar for prefetched sources |
| **Hash strategy** | Reused verbatim for crates.io — `Cargo.lock`'s hex SHA-256 `checksum` copied straight into Nix's hex `sha256` field, no SRI/base32 conversion. Prefetched (`nix-prefetch-url`/`-git`) only when the lockfile has none (git deps always; alternate registries) |
| **Nix build mechanism** | One derivation per crate via nixpkgs' `buildRustCrate` |

## Input

`crate2nix generate` shells out to `cargo metadata` (the same machinery `cargo build` itself uses) rather than parsing `Cargo.toml` by hand — it inherits Cargo's real feature unification, `cfg()` filtering, and optional-dependency activation for free. `crate2nix/src/lock.rs` separately parses `Cargo.lock` to build a `PackageId → checksum` map, matching each entry to the corresponding `cargo_metadata::Package` on `(name, source, version)`. A checksum literal `"<none>"` is treated as absent. Source kind is read straight from Cargo's own `source` string:

| Cargo `source` | crate2nix variant |
|---|---|
| `registry+https://.../crates.io-index` | `CratesIo { sha256 }` |
| `sparse+...` | `Registry { sha256 }` (alternate/private registry) |
| `git+...` | `Git { url, rev, sha256 }` |
| absent / unresolvable git rev | `LocalDirectory { path }` (workspace/path deps) |

There is no "URL tarball" kind in Cargo's own model (unlike Gossamer's `{ url, sha256 }`), so crate2nix has nothing analogous to build there.

## Output and the checksum → hash path

The generated `Cargo.nix` is a plain attribute set per crate (`internal.crates.<packageId>`) — not itself a derivation. `internal.buildRustCrateWithFeatures` walks the graph at eval time doing feature-union resolution, then calls nixpkgs' `buildRustCrateForPkgs` to actually produce a derivation. For a crates.io source the template emits only `sha256 = "<hex-checksum>";`; nixpkgs' `buildRustCrate` internally calls `pkgs.fetchcrate`, builds the download URL, and passes that hash straight through to `fetchzip`/`fetchurl`. **This is a direct reuse, not a conversion** — Cargo's `checksum` is already a plain hex SHA-256 of the `.crate` tarball, and Nix's fetchers accept plain hex SHA-256 natively alongside the newer SRI form. Where `Cargo.lock` has no checksum, crate2nix prefetches itself (`nix-prefetch-url`/`nix-prefetch-git`) and records the result in `crate-hashes.json`; alternate-registry prefetching is instead reimplemented *inside the generated Nix* (a `registryUrl` helper reading the registry's `config.json` `dl` template per the Cargo sparse-registry protocol).

## Nix-side mechanism

Per-crate derivations, not a monolithic build — the core selling point over naersk's "build the whole dependency closure in one derivation" approach. Two generation strategies trade off against import-from-derivation (IFD): **manual** (`crate2nix generate`, `Cargo.nix` checked in — full parallelism, but can go stale) vs. **auto/`tools.nix`** (IFD-evaluated, always in sync, some parallelism cost). `build.rs` and proc-macros are delegated entirely to nixpkgs' `buildRustCrate`, run as an ordinary build-phase step inside the same sandboxed, no-network derivation. Features are resolved at Nix-evaluation/build time, not generation time, so `.override { features = [...]; }` works without regenerating. Workspaces expose every member as `workspaceMembers.<name>.build`; a known restriction is that each crate only sees its own source directory at build time, not sibling workspace directories.

## Why the tool exists

Cargo wants to reach the network (crates.io, git remotes) during `cargo build`; Nix derivations are sandboxed for reproducibility. Running plain `cargo build` in an ordinary derivation fails outright. crate2nix's answer: walk Cargo's own resolved graph, get a content hash for every external source *before* the sandboxed build starts (from `Cargo.lock` or its own prefetch), turn each into a small Nix fixed-output fetch, so the compile derivations never need network — `rustc` only ever sees a local, already-fetched `src`. (`carnix`, crate2nix's predecessor, reportedly "failed to generate correct builds" for real projects — evidence this is a hard problem, not a trivial wrapper.)

## Limitations

- Tests are still marked experimental; a failing test fails the whole derivation unless silenced via hooks.
- Target-specific (`cfg(target_...)`) features don't resolve automatically.
- `build.rs` scripts that themselves need network access will fail under the sandboxed model — inherent to the approach, not a crate2nix bug.
- Alternate/sparse-registry protocol handling lives partly in generated Nix rather than the maintained Rust codebase — more surface area for drift.

## Relevance to `gossamer2nix`

`[rust-bindings]`'s `Crates`/`Git`/`Path` variants map directly onto crate2nix's source handling. But crate2nix needs a real `Cargo.lock`/`Cargo.toml` — it calls `cargo metadata`, it doesn't invent a graph from a manifest fragment. A `gossamer2nix` adapter would need to first materialize a synthetic Cargo crate from the `[rust-bindings]` table before crate2nix (or logic modeled on it) has anything to resolve against. The `Prebuilt` variant (archive keyed by ABI version) bypasses Cargo entirely — a plain `fetchurl`-shaped fetch, not a crate2nix-shaped problem.
