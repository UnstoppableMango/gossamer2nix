# `naersk`: Bridging Cargo into Nix

Read 2026-07-26 (`master`, commit `9aa07bb0`) as prior art for building a Cargo dependency graph in Nix, since Gossamer's `[rust-bindings]` FFI section (`../DEPS.md` §3.1) needs "normal Cargo/crates.io tooling." Sources: `default.nix`, `lib.nix`, `build.nix`, `config.nix`, `builtins/default.nix`, `README.md`, issue tracker.

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/naersk](https://github.com/nix-community/naersk) (`master`) |
| **Ecosystem** | Rust / Cargo |
| **Maintenance status** | Orphaned — original author stepped down (issue #320, Dec 2023, still unanswered); light drive-by fixes since. Secondary sources cite [`crane`](https://github.com/ipetkov/crane) as the community's spiritual successor |
| **Ecosystem input** | `Cargo.lock`, read directly at Nix-eval time via pure `builtins.fromTOML` — no separate generated file, no IFD |
| **Generated output** | None checked in — nothing is generated/committed; the lockfile is consumed fresh on every build |
| **Hash strategy** | Reused verbatim — `Cargo.lock`'s `checksum` field becomes the `fetchurl` `sha256` for every crates.io crate, one FOD each. Git deps use `builtins.fetchGit` keyed on the pinned revision (no separate hash needed) |
| **Nix build mechanism** | Two-stage split: a deps-only derivation + the real app derivation, linked by copying a compiled `target/` tree between them |

## Input

`naersk` requires `Cargo.lock` to exist on disk next to `Cargo.toml` — no separate lockfile, no generated Nix file to keep in sync:

```nix
cargolock = if builtins.pathExists cargolock-file then readTOML (cargolock-file)
  else throw "Naersk requires Cargo.lock to be available in root...";
```

Parsing avoids IFD by default: `builtins.fromTOML` runs in pure Nix evaluation (falling back to a `remarshal`-via-derivation shim only on old/buggy Nix). This is naersk's core differentiator vs. crate2nix: the lockfile *is* the only source of truth, always fresh, with nothing that can drift out of sync.

## Output and hash story

No per-crate derivations are pre-generated. Instead, `lib.nix`'s `mkVersions` reads whichever checksum form `Cargo.lock` has (modern per-package `checksum`, or the legacy `[metadata]` table) and turns each `{name, version, sha256}` into its own fixed-output derivation:

```nix
unpackCratesIoDependency = { name, version, sha256 }:
  let crate = fetchurl { inherit sha256; url = "${cratesDownloadUrl}/${name}/${version}/download"; };
  in runCommandLocal "unpack-${name}-${version}" { } ''tar -xzf ${crate} -C $out; ...'';
```

All per-crate unpack derivations are symlink-joined into one tree, wired into a generated Cargo config as a `[source.crates-io] replace-with` **directory source** — Cargo's own [source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html) mechanism, not `cargo vendor`. That config is copied to `$CARGO_HOME/config.toml`, so `cargo build` resolves every dependency from the Nix store instead of the network. Git dependencies are fetched with `builtins.fetchGit`, keyed by the pinned commit SHA already in `Cargo.lock` — no separate Nix hash needed since the revision itself is the integrity guarantee.

## Nix-side mechanism: dummy-source deps/app split

`buildPackage` produces up to two derivations: **`buildDeps`** compiles only the dependency closure, against a synthesized "dummy" source tree (stub `build.rs`/`lib.rs`, trimmed `Cargo.toml`s with bins/examples/tests stripped) so real application code never compiles here; its output (`target.tar.zst`) is naersk's caching unit. **`buildTopLevel`** unpacks that tree into its own `target/` before running `cargo build` on the real `src`, so Cargo sees pre-built `.rlib`s for every dependency. `singleStep = true` disables the split for cases needing `overrideAttrs` on the final derivation (which otherwise only patches the app half, not the separately-built deps).

## Why the tool exists, and vs. crate2nix

Before tools like this, packaging Rust for Nix meant either running `cargo build` with impure network access, or hand-generating a full per-crate Nix derivation graph (crate2nix's approach). naersk asks a narrower question: what's the least Nix code needed to make `cargo build` itself sandboxable? Its answer is Cargo's own `[source] replace-with` config plus a two-stage cache split — no codegen step, nothing to regenerate after `cargo add`/`cargo update`.

| | naersk | crate2nix |
|---|---|---|
| Generated/checked-in file | none | `Cargo.nix` |
| Derivation granularity | ~2 (deps, app) + 1 FOD per crate | 1 per crate |
| Rebuild on dep bump | whole deps derivation invalidates | only the changed crate rebuilds |
| Setup burden | `buildPackage { src = ./.; }`, done | `crate2nix generate` (or IFD) on every lockfile change |

## Limitations

- **`overrideAttrs` footgun**: only patches the final app derivation, not the separately-built deps derivation — use `buildPackage`'s own override hooks or `singleStep = true`.
- **Dummy-source generation is a real friction source**: issue tracker shows workspace edge cases (`[lib]` not copied, submodules in git deps, test-mode running on the dummy src) tied to the stub-file substitution not perfectly mimicking real project shape.
- **`rust-toolchain` file ignored by default** unless `rustc`/`cargo` are explicitly wired in via `nixpkgs-mozilla`/`rust-overlay`.
- **Cross-compilation** has open friction; no confirmed first-class story.
- `Cargo.lock` is mandatory and must be tracked/staged — naersk throws immediately if it's missing or `.gitignore`d.

## Relevance to `gossamer2nix`

A `Crates`/`Git`/`Path` `[rust-bindings]` entry, once scaffolded into a `Cargo.toml`, is exactly what `naersk.buildPackage` already knows how to turn into a sandboxed, `Cargo.lock`-driven derivation — no separate fetcher/hash database needed, since naersk leans on checksums already in the relevant lockfile. naersk's zero-codegen, read-the-lockfile-directly philosophy mirrors Gossamer's own "`project.lock` is the single source of truth" model, avoiding a second lockfile-drift surface the way a crate2nix-style committed `Cargo.nix` would introduce. Given the maintenance caveat above, treat naersk as known-good, simple prior art rather than a long-term dependency without also evaluating `crane`.
