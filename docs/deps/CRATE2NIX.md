# `crate2nix`: Bridging Cargo into Nix

This document was compiled by reading the [nix-community/crate2nix](https://github.com/nix-community/crate2nix) repository directly — its Rust source (`crate2nix/src/{lock,resolve,prefetch,metadata}.rs`), its Nix code-generation templates (`crate2nix/templates/Cargo.nix.tera`, `crate2nix/templates/nix/crate2nix/default.nix`), and its documentation site (`docs/src/content/docs/**`, published at [nix-community.github.io/crate2nix](https://nix-community.github.io/crate2nix/)) — plus the relevant parts of `nixpkgs` (`pkgs/build-support/rust/{fetchcrate.nix,build-rust-crate/{default.nix,build-crate.nix}}`) that `crate2nix`-generated code actually calls into.
All reads were done via GitHub's raw-content and contents APIs against the `master` branch, **2026-07-26**.

It exists to ground this repo's eventual handling of Gossamer's `[rust-bindings]` FFI dependency graph (see [`../DEPS.md`](../DEPS.md) §3.1) in how the most established prior-art tool actually bridges Cargo into Nix, rather than assumptions from marketing copy.

**This reflects one point-in-time reading of upstream `master` and may drift.** `crate2nix` is maintained but slow-moving — its 0.15.0 release [shipped after almost two years](https://discourse.nixos.org/t/crate2nix-0-15-released/74806), and the maintainer has publicly asked for a co-maintainer. Re-verify field names, template contents, and CLI flags against current upstream source before depending on exact details in code.

---

## Input / Output at a glance

**Input** (ecosystem-native, Cargo's own artifacts):
- `Cargo.toml` (workspace/package manifest — read indirectly via `cargo metadata`, not parsed by `crate2nix` itself)
- `Cargo.lock` — specifically: `[[package]]` `name`, `version`, `source` (the `registry+`/`sparse+`/`git+`/absent-for-path URL-ish string), and `checksum` (the crates.io SHA-256 hex digest of the `.crate` tarball, or the older `[metadata]` `"checksum <name> <version> (<source>)"` legacy form)
- Live output of `cargo metadata` (resolved dependency graph, feature graph, target info, build scripts, workspace membership — the same data `cargo build` itself would compute)
- For alternate/private registries: the registry's `config.json` (its `dl` download-URL template, per the [Cargo registry-index protocol](https://doc.rust-lang.org/cargo/reference/registry-index.html#index-configuration))
- Optionally, out-of-tree source declarations in a hand-written `crate2nix.json` (crates.io-by-name+version, git-by-URL+rev, or raw Nix expressions), for building a crate that isn't the checked-out project itself

**Output** (Nix-consumable):
- A generated `Cargo.nix` (or a directory containing an IFD-produced `default.nix`) — a self-contained Nix expression with **one derivation-producing attribute set per crate node in the resolved dependency graph**, keyed by Cargo package ID, wired together via `internal.crates.<packageId>.dependencies[].packageId` references
- A `crate-hashes.json` sidecar recording Nix `sha256` hashes for sources it had to prefetch itself (non-crates.io / non-checksum sources)
- Each per-crate node carries a `sha256` (crates.io) or a `src = fetchurl {...}` / `src = pkgs.fetchgit {...}` (alternate registry / git) attribute that nixpkgs' `buildRustCrate` (via `pkgs.fetchcrate`/`fetchzip`/`fetchgit`) turns into an actual fixed-output-derivation fetch — the generated `Cargo.nix` itself does not fetch anything; it only supplies the hash and coordinates
- `rootCrate.build` (single binary/library projects) or `workspaceMembers.<name>.build` (workspaces) — the buildable derivations a consumer actually invokes

---

## 1. What input does `crate2nix` consume?

`crate2nix generate` does **not** hand-parse `Cargo.toml`. Per the documented phase list (`docs/src/content/docs/70_design/10_structure_and_phases.md`):

1. **cargo metadata** — it shells out to `cargo metadata` via the `cargo_metadata` crate, which is the same machinery `cargo build` uses internally. This means `crate2nix` inherits Cargo's actual dependency resolution (feature unification, platform `cfg()` filtering, optional-dependency activation) rather than re-implementing it — a deliberate choice to stay correct as Cargo's resolver evolves.
2. **indexing metadata** — joins metadata's package/node graph by package ID into `metadata::IndexedMetadata`.
3. **resolving** — builds `resolve::CrateDerivation` records, one per package, carrying everything the Nix side needs (name, version, edition, targets, dependency edges, features, and a `ResolvedSource`).
4. **pre-fetching** — fills in Nix `sha256` hashes for sources that need them (§2 below).
5. **rendering** — feeds everything through the `build.nix.tera` (actually `Cargo.nix.tera`) template.

### `Cargo.lock` fields it actually reads

`crate2nix/src/lock.rs` parses `Cargo.lock` as TOML into an `EncodableResolve { package: Vec<EncodableDependency>, root, metadata }`, where each `EncodableDependency` has `name`, `version`, `source: Option<String>`, `checksum: Option<String>`. Its only real job (`get_hashes_by_package_id`) is building a `HashMap<PackageId, checksum>` by matching each lock entry to the corresponding `cargo_metadata::Package` on `(name, source, version)`. Checksums come from either:
- the modern per-package `checksum` field, or
- for older `Cargo.lock` v1/v2 files, the legacy `[metadata]` table entry keyed `"checksum <name> <version> (<source>)"`.

A checksum literal `"<none>"` (Cargo's marker for "known not to have one") is treated as absent.

**Critically, `crate2nix` never independently re-derives this checksum — it treats whatever Cargo already verified as ground truth**, then only falls back to prefetching (`nix-prefetch-url` / `nix-prefetch-git`) for packages the lockfile didn't cover (`crate2nix/src/prefetch.rs::prefetch`, which checks `from_lock_file` before `old_prefetched_hashes` before falling back to an actual network prefetch).

### How it distinguishes source kinds

`resolve.rs::ResolvedSource::new` switches on `cargo_metadata::Package.source` (the same `source` string Cargo puts in `Cargo.lock`):

| Cargo's `source` value | `crate2nix` variant | Notes |
|---|---|---|
| `is_crates_io()` (i.e. `registry+https://github.com/rust-lang/crates.io-index`) | `ResolvedSource::CratesIo { name, version, sha256 }` | The common case; hash filled from `Cargo.lock` checksum or prefetch |
| starts with `sparse+` | `ResolvedSource::Registry { registry, name, version, sha256 }` | Alternate/private sparse registries (post-2023 Cargo registry protocol) |
| starts with `git+` | `ResolvedSource::Git { url, rev, ref, sha256 }` | Parses `?branch=`/`?rev=` query params or the URL fragment for the commit; requires a resolvable revision or it falls back to local-directory with a warning |
| `None`, or a git source with **no `git+` prefix or no discoverable revision** | `ResolvedSource::LocalDirectory { path }` | Path (workspace-member/path) dependencies — also the fallback for anything it can't parse, emitted with an explicit `WARNING:` to stderr |

There is no first-class "URL tarball" kind in Cargo's own model (unlike Gossamer's `{ url, sha256 }` dependency kind — see `../DEPS.md` §3), so `crate2nix` has nothing analogous to build there.

---

## 2. What does it produce, and how does the checksum become a Nix hash?

The generated `Cargo.nix` is one big `rec { rootCrate = ...; workspaceMembers = ...; internal.crates.<packageId> = {...}; }` expression (`crate2nix/templates/Cargo.nix.tera`). Each `internal.crates.<packageId>` block is a plain attribute set — **not itself a derivation** — carrying `crateName`, `version`, `edition`, `dependencies`/`buildDependencies`/`devDependencies` (each with `packageId`, optional `rename`, `features`, `target` predicate), `features`, `resolvedDefaultFeatures`, and a source attribute. `internal.buildRustCrateWithFeatures` (from the included `nix/crate2nix/default.nix` fragment) walks this graph at Nix-evaluation time doing feature-union resolution, then calls `buildRustCrateForPkgs pkgs { ...crate config... }` per node — that's where an actual derivation is produced, by nixpkgs' `buildRustCrate`, not by `crate2nix` itself. `crate2nix` supplies the config; nixpkgs supplies the build logic.

### The checksum → hash path, concretely

For a `CratesIo` source the template emits only:

```nix
sha256 = "<hex-checksum-from-Cargo.lock-or-prefetch>";
```

No `src` — nixpkgs' `buildRustCrate` internally calls `pkgs.fetchcrate` (`pkgs/build-support/rust/fetchcrate.nix` in nixpkgs), which builds the download URL as `"${registryDl}/${crateName}/${version}/download"` against `registryDl = "https://static.crates.io/crates"` by default, and passes the `sha256` straight through to `fetchzip`/`fetchurl`.

**This is a direct reuse, not an SRI/base32 conversion.** Cargo's `checksum` field is already a plain SHA-256 hex digest of the `.crate` tarball bytes; Nix's `fetchurl`/`fetchzip` `sha256` argument accepts a plain hex-encoded SHA-256 directly (the legacy pre-SRI hash format Nix has always supported alongside the newer `hash = "sha256-<base64>";` SRI form). `crate2nix` does no cryptographic re-encoding — it copies the string. (It's the same value; Cargo's registry protocol and Nix's fixed-output-derivation model both happen to standardize on SHA-256, so the digest transfers as-is, just reinterpreted from "hex string" to "hex string" rather than needing base64/SRI translation. `nix hash convert`/`nix-hash --to-sri` would only be needed if you wanted the SRI spelling for readability — functionally it's the identical hash either way.)

For sources where `Cargo.lock` has no checksum (git dependencies always; sparse-registry/alternate-registry packages; anything `--offline`/pre-lockfile), `crate2nix` shells out itself:
- **crates.io fallback**: `nix-prefetch-url <static.crates.io URL> --name <crate>-<version>` (`prefetch.rs::CratesIoSource::prefetch`)
- **git**: `nix-prefetch-git --url <url> --fetch-submodules --rev <rev> [--branch-name <ref>]`, parsing the JSON result's `sha256` field (`prefetch.rs::GitSource::prefetch`) — this is also how `crate2nix` picked up submodule support (0.7+, per the "Known Restrictions" doc)
- **alternate registries**: `RegistrySource::prefetch` is `unimplemented!()` in the Rust code — that path is instead handled *inside the generated Nix* via a `registryUrl` helper function (`templates/nix/crate2nix/default.nix`) that reads the registry's `config.json` `dl` template (fetched once per registry into `registries.<url>` via `fetchurl`) and substitutes `{crate}`/`{version}`/`{prefix}`/`{lowerprefix}`/`{sha256-checksum}` placeholders per the [Cargo sparse-registry index-configuration spec](https://doc.rust-lang.org/cargo/reference/registry-index.html#index-configuration) — i.e. it reimplements a slice of Cargo's own registry protocol in Nix, not in Rust.

Any newly-prefetched hash is written back to a `crate-hashes.json` file (only for hashes it actually had to fetch — existing `Cargo.lock` checksums are *not* duplicated into it), meant to be checked into version control alongside `Cargo.lock` so re-generation is reproducible without re-hitting the network (`docs/40_external_sources/20_generating_for_fetched_sources.md`).

---

## 3. The actual Nix-side build mechanism

**Per-crate derivations, not a monolithic build.** Every node in the resolved graph becomes its own `buildRustCrate`-produced derivation (visible as `internal.crates.<packageId>` config feeding `buildRustCrateForPkgs`), which is the whole point: `nix build` only rebuilds the crates whose inputs actually changed, and independent crates build in parallel as separate derivations — this is the tool's core selling point over `naersk`-style "build the whole dependency closure in one derivation" approaches (`docs/70_design/90_inspiration.md`).

**Two generation strategies**, both producing the same shape of output, trading off differently against Nix's [import-from-derivation (IFD)](https://nixos.org/manual/nix/stable/language/import-from-derivation) feature:
- **Manual** (`crate2nix generate`, run by a developer/CI, `Cargo.nix` checked into version control): no IFD, full build parallelism, but `Cargo.nix` goes stale if `Cargo.lock`/`Cargo.toml` change without re-running `generate`.
- **Auto / `tools.nix`** (`generatedCargoNix`/`appliedCargoNix`, evaluated inside a derivation at Nix-evaluation time via IFD): always in sync with `Cargo.lock` since it re-runs `crate2nix` as part of evaluation, but IFD can disable some build parallelism, and per `docs/70_design/50_tools_nix.mdx` it works by having Nix itself read `Cargo.lock`'s locked versions/hashes, fetch the dependencies *without introducing impurities* into a vendored folder, and run `crate2nix generate` **offline** inside that sandboxed derivation to produce the `Cargo.nix` that gets imported.

There's also an **experimental JSON output** mode (`crate2nix generate --format json`) that does feature-expansion/platform-filtering/optional-dependency resolution on the Rust side instead of at Nix-evaluation time, producing a smaller, pre-resolved file — a performance optimization for very large graphs, still explicitly experimental.

### Build scripts (`build.rs`)

Not something `crate2nix` implements itself — it delegates entirely to nixpkgs' `buildRustCrate` (`pkgs/build-support/rust/build-rust-crate/{default.nix,build-crate.nix}`), which compiles and runs a crate's `build.rs` as a normal build-phase step inside the same sandboxed derivation (no separate derivation for the build script), setting the usual Cargo build-script environment (`OUT_DIR`, `CARGO_MANIFEST_DIR`, etc.) and honoring the crate's `links` field for `-sys`-crate native-library linkage metadata (`crate.links` is threaded through the template into `crateLinks`). `crate2nix` surfaces the `build.rs` source path in the generated crate config only when it's non-default (`crate.build.src_path` in `Cargo.nix.tera`).

### Proc-macros

Also delegated to `buildRustCrate` — `crate2nix` just sets `procMacro = true;` on the relevant crate node when `crate.proc_macro` is true (from `cargo metadata`'s crate-type info); nixpkgs' build logic knows to build/link proc-macro crates as loadable compiler plugins (`--extern proc_macro`, `dylib`-style crate type) rather than as ordinary libraries.

### Features

Resolved **at Nix-evaluation/build time, not at generation time**. `crate2nix generate` by default emits the crate's default-feature dependency edges; `internal.buildRustCrateWithFeatures` in the generated file does the actual feature-union graph walk when you build, so `rootFeatures`/`.override { features = [...]; }` can change what gets built without regenerating `Cargo.nix` — documented explicitly as a deliberate choice because "even though features in Rust are meant to be additive, in reality they are often not," so per-binary feature-set precision avoids spurious rebuild/incompatibility problems (`docs/40_external_sources/20_generating_for_fetched_sources.md` "Feature resolution" section, `docs/30_building/20_choosing_features.md`). `crate2nix generate --all-features` / `--no-default-features --features "..."` exist to control what's even representable in the generated file.

### Workspaces

`cargo_metadata` reports full workspace membership, and `crate2nix` exposes every workspace member as `workspaceMembers.<name>.build` (there is no single `rootCrate` for a workspace — that attribute only exists for single-binary/library non-workspace projects) (`docs/30_building/10_building_binaries.md`). A known restriction (`docs/90_reference/20_known_restrictions.md`): each crate only has access to its own source directory during the build, not sibling directories in the same workspace, which can bite workspace-relative-path tricks that work fine under plain `cargo build`.

---

## 4. Why does this tool need to exist at all?

The gap: **Cargo wants to reach the network** (crates.io index/downloads, git remotes) as part of `cargo build`'s normal operation, while **Nix derivations build in a network-sandboxed environment** by design (so builds are reproducible and cacheable by content hash). Running plain `cargo build` inside an ordinary Nix derivation fails outright — there's no crates.io access — unless you either (a) turn the whole thing into a single fixed-output derivation that pre-vendors all dependencies (giving up Nix's per-input caching/rebuild granularity and the ability to build derivations sequentially with real content-addressed inputs), or (b) teach Nix each individual crate's source and content hash up front, as separate fixed-output fetches, and only run `cargo`/`rustc` for the actual compilation inside a normal (sandboxed, no-network) derivation.

`crate2nix` implements (b): it walks Cargo's own resolved dependency graph, gets a content hash for every external source *before* the sandboxed build starts (either straight from `Cargo.lock`'s existing checksum or via its own prefetch step), and turns each one into a small Nix fixed-output fetch, so the actual compile derivations never need network access at all — Nix fetches the sources up front (verified against the declared hash, exactly like any other Nix fixed-output derivation), and `rustc` only ever sees a local, already-fetched `src`.

The `carnix` → `crate2nix` history documented in `docs/70_design/90_inspiration.md` is itself evidence this is a hard problem worth a dedicated tool: `carnix` attempted the same crate-granularity approach earlier and, per the same maintainer's account, "failed to generate correct builds" for real projects, motivating a rewrite rather than a patch.

---

## 5. Limitations and gotchas (as documented)

From `docs/90_reference/20_known_restrictions.md` and other pages read above:

- **Tests are still marked experimental** even though they've worked for a while (`runTests = true;` on the crate's `.build.override`); failing tests fail the whole derivation build unless silenced via `testPreRun`/`testPostRun` hooks.
- **Target-specific (`cfg(target_...)`) features don't resolve automatically** ([issue #129](https://github.com/nix-community/crate2nix/issues/129)) — workaround is to enable them manually.
- **A crate only sees its own source directory at build time**, not sibling workspace directories (crate2nix issue #17) — a real gotcha for workspaces that lean on relative paths outside a single crate's tree.
- **Alternate/sparse registries reimplement part of Cargo's registry protocol in generated Nix** (the `registryUrl` template-substitution function), rather than in the maintained Rust codebase — more surface area for drift if Cargo's registry protocol changes.
- **`build.rs` impurity**: because build scripts run inside the sandboxed, no-network derivation, any `build.rs` that itself tries to reach the network (e.g. downloading a vendored asset, phoning home for a version check) will fail under `crate2nix`/Nix even though it might work fine under plain `cargo build` with network access — not something `crate2nix` can paper over, since it's an inherent Nix-sandbox vs. Cargo-ecosystem-convention gap, not a bug in `crate2nix` itself. (Not explicitly called out as a bullet in the "Known Restrictions" page, but a direct, structural consequence of the crate-by-crate-in-sandboxed-derivations model described throughout the docs and confirmed by reading `buildRustCrate`'s build-phase code, which offers no network escape hatch.)
- **Maintenance bus factor**: as of the 0.15.0 release notes, the primary maintainer has said they lack time to meaningfully advance the project and is looking for a co-maintainer — worth weighing before taking a hard dependency on `crate2nix`'s pace of fixes for a `gossamer2nix` implementation.
- **Nixpkgs-version coupling**: `crate2nix` depends on relatively recent `buildRustCrate` features in nixpkgs; check `nix/sources.json` in its repo for the pinned nixpkgs version it's tested against before assuming compatibility with an arbitrary nixpkgs revision.

---

## Relevance to `gossamer2nix`'s `[rust-bindings]` handling

`../DEPS.md` §3.1 documents Gossamer's `[rust-bindings]` manifest section as a **separate dependency graph from `[dependencies]`**, resolved by Cargo's own tooling once a binding is scaffolded, with five variants (`Path`, `Git`, `Crates`, `Src`, `Prebuilt`). Of these, `Crates`, `Git`, and (once scaffolded to a real Cargo crate directory) `Path` bindings are exactly the shape of dependency `crate2nix` already knows how to turn into hashed, sandboxable Nix fetches — a crates.io-sourced binding maps directly onto `crate2nix`'s `CratesIo` source handling (§1–2 above), a git-sourced binding onto its `Git` source handling (including submodules), and a path-sourced binding onto its `LocalDirectory` handling.

Two things worth carrying into a `gossamer2nix` design:

1. **`crate2nix` needs a real `Cargo.lock`/`Cargo.toml` to operate on** (it calls `cargo metadata`, it doesn't invent a graph from a manifest fragment). Since `[rust-bindings]` is described in `../DEPS.md` as inline TOML inside `project.toml` rather than a standalone Cargo project, a `gossamer2nix` adapter would first need to materialize a synthetic Cargo crate (a scaffolded `Cargo.toml`/`Cargo.lock` pair reflecting the `[rust-bindings]` table's `Crates`/`Git`/`Path` entries) before `crate2nix` — or logic modeled directly on it — has anything to resolve against. This mirrors what `gos`'s own FFI scaffolding step presumably already does at build time (not confirmed from source read for this document; worth checking `gossamer-cli`/`gossamer-driver` FFI codegen paths the way `../DEPS.md` §12 checked the `.gos` dependency path).
2. **The `Src`/`Prebuilt` binding variants have no Cargo-native analogue** `crate2nix` covers: `Src` inlines a raw Cargo-deps TOML fragment for a single file (would still resolve through the same synthetic-`Cargo.toml` path once expanded), but `Prebuilt` (a pre-built static archive keyed by ABI version) bypasses Cargo/`crate2nix` entirely — that's a plain content-addressed fetch (`fetchurl`-shaped), not a `crate2nix`-shaped problem at all.

In short: `crate2nix` (or its underlying approach — hash-first, per-crate, `buildRustCrate`-backed fetches) is the natural tool for the `Crates`/`Git`/`Path` slice of `[rust-bindings]`, but a `gossamer2nix` deps-lock generator still owns the step of turning `[rust-bindings]` TOML into something Cargo-shaped enough for `crate2nix` (or equivalent) to consume in the first place.

---

## Sources

- [nix-community/crate2nix](https://github.com/nix-community/crate2nix), `master` branch, read 2026-07-26:
  - `crate2nix/src/lock.rs` (`Cargo.lock` parsing, checksum extraction)
  - `crate2nix/src/resolve.rs` (`ResolvedSource` enum and per-source-kind resolution, `CratesIoSource::url()`)
  - `crate2nix/src/prefetch.rs` (`nix-prefetch-url`/`nix-prefetch-git` fallback logic, `crate-hashes.json` handling)
  - `crate2nix/templates/Cargo.nix.tera` (generated-file structure, per-crate `sha256`/`src` emission)
  - `crate2nix/templates/nix/crate2nix/default.nix` (`buildRustCrateWithFeatures`, `registryUrl`, `sourceFilter`)
  - `docs/src/content/docs/20_generating/{10_generating.md,20_auto_generating.mdx}`
  - `docs/src/content/docs/30_building/{10_building_binaries.md,20_choosing_features.md,30_crateOverrides.md,40_tests.md}`
  - `docs/src/content/docs/35_toolchains/{10_custom_toolchains.md,20_using_a_rust_overlay.md}`
  - `docs/src/content/docs/40_external_sources/{10_building_fetched_sources.md,20_generating_for_fetched_sources.md}`
  - `docs/src/content/docs/70_design/{10_structure_and_phases.md,50_tools_nix.mdx,90_inspiration.md}`
  - `docs/src/content/docs/90_reference/{10_runtime_dependencies.md,20_known_restrictions.md}`
- [nixpkgs](https://github.com/NixOS/nixpkgs), `master` branch, read 2026-07-26:
  - `pkgs/build-support/rust/fetchcrate.nix` (crates.io URL construction, `fetchzip`/`fetchurl` hash passthrough)
  - `pkgs/build-support/rust/build-rust-crate/{default.nix,build-crate.nix}` (`buildRustCrate`, build-script/proc-macro handling)
- [Cargo Book: Registry Index — Index Configuration](https://doc.rust-lang.org/cargo/reference/registry-index.html#index-configuration) (the `dl` template protocol `registryUrl` reimplements)
- [Crate2nix 0.15 released — NixOS Discourse](https://discourse.nixos.org/t/crate2nix-0-15-released/74806) (maintenance-status context)

This reflects a point-in-time reading via GitHub's raw-content/contents APIs; no local clone of either repo exists in this workspace. The `build.rs`-network-impurity claim in §5 is a structural inference from reading `buildRustCrate`'s build-phase code and the sandboxed-derivation model described throughout the docs, not a direct quote from a "known restrictions" bullet — flagged there as such; treat it as **very likely but not verbatim-documented**. Re-verify current template/source-file contents before depending on exact Nix attribute names in code, since this project's own docs note the internal (`internal.*`) attributes are explicitly not held to a stability guarantee across releases.
