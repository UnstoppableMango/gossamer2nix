# `naersk`: bridging Cargo into Nix

This document was compiled by reading source directly from [nix-community/naersk](https://github.com/nix-community/naersk) (`master` branch, commit `9aa07bb0`, pushed 2026-06-23) — specifically `default.nix`, `lib.nix`, `build.nix`, `config.nix`, `builtins/default.nix`, and `README.md` — plus the repo's GitHub issue tracker and API metadata, read on 2026-07-26.
It exists to give `gossamer2nix` prior art for building a Cargo dependency graph inside Nix, since Gossamer's `[rust-bindings]` FFI section (`docs/DEPS.md` §3.1) is "a separate dependency graph a nix adapter needs to handle via normal Cargo/crates.io tooling, not [the Gossamer resolver] pipeline."

**This reflects one point-in-time reading of upstream `master` and may drift.** Re-verify against current upstream source before depending on exact details in code. Unconfirmed items are flagged explicitly below.

---

## 0. Is it still the canonical, maintained repo?

Yes for canonical; **maintenance status is a real yellow flag.**

- `nix-community/naersk` is the repo everything else (nixpkgs discussions, blog posts, forks) points to. It is **not archived** (`isArchived: false` via `gh repo view`), has 1001 stars, 100 forks, 63 open issues, MIT-licensed.
- It received a real commit on 2026-06-23 (`#397`, "Fix workspace version in single Cargo.toml with workspace = true"), merged by `nmattia` — the original author.
- **However**, `nmattia` opened issue [#320 "Looking for a maintainer"](https://github.com/nix-community/naersk/issues/320) on 2023-12-19: *"I'm gonna be stepping down as maintainer, so Naersk needs a new one."* That issue has zero replies and is still open as of 2026-07-26.
- In practice: the project has had **light, drive-by maintenance since late 2023** — small fixes from community contributors (`hannahfluch`, `link2xt`, `Tomoaki Kawada`, `dly` among recent committers) plus occasional merges by the original, formally-departed author, rather than active feature development or a designated maintainer.
- Multiple secondary sources (a 2025 devenv.sh blog post, a "Bleeding-edge Rust on NixOS" practitioner's guide) describe **[`ipetkov/crane`](https://github.com/ipetkov/crane)** as naersk's "spiritual successor" and the currently-recommended default for new projects, with naersk kept around mostly for existing users and crate2nix reserved for workspaces with very large dependency trees where fine-grained per-crate caching matters most. **This framing is from secondary/community sources, not from naersk's own docs or maintainers directly — flagged as unconfirmed**, but it's a strong enough signal that a `gossamer2nix` design decision should at least glance at `crane` before committing to naersk.

---

## 1. Input

**`naersk` requires `Cargo.lock` to exist on disk next to `Cargo.toml` — no separate lockfile, no generated Nix file.** From `config.nix`:

```nix
cargolock =
  let
    cargolock-file = root + "/Cargo.lock";
  in
  if builtins.pathExists cargolock-file then
    readTOML (cargolock-file)
  else
    throw "Naersk requires Cargo.lock to be available in root. Check that it is not in .gitignore and stage it when using git to filter sources (which flakes does)";
```

- `buildPackage { src, root, ... }`: `root` (defaulting to `src`) is where naersk looks for `Cargo.toml`/`Cargo.lock`; `src` is what's actually copied into the build sandbox. They can differ (e.g. a `root` with just manifests for a monorepo).
- **Parsing avoids IFD**: `readTOML` (`config.nix:265`, backed by `builtins/default.nix`) uses Nix's native `builtins.fromTOML` when `usePureFromTOML = true` (the default since Nix 2.3). Only as a fallback for old/buggy Nix does it shell out to the Python `remarshal` tool inside a derivation and read the result with `builtins.fromJSON` — which *would* be IFD. So by default, everything is parsed in pure Nix evaluation, which is why the README can claim: *"If you're using Hydra, you can rely on Naersk as well because it doesn't use IFD — all the parsing happens directly inside Nix code."*
- `additionalCargoLock`: an optional second `Cargo.lock`-shaped file merged in, for cases (e.g. patched/vendored crates) where the primary lockfile doesn't have everything.
- Workspaces: `config.nix` detects `isWorkspace` from `toplevelCargotoml ? workspace` and walks workspace members to build a `cargotomls` list (`{ name, toml }` pairs), one per member — but the *lockfile* is still singular, the workspace-wide `Cargo.lock`.
- **This is naersk's core differentiator vs. crate2nix**: naersk reads `Cargo.lock` directly, at eval time, every build. There is no `naersk generate`, no `Cargo.nix` (or equivalent) committed to the repo that can drift out of sync with `Cargo.lock` — the lockfile *is* the only source of truth, always fresh.

## 2. Output

`buildPackage` produces (up to) **two `stdenv.mkDerivation` derivations**, wired together, not one derivation per crate:

1. **`buildDeps`** (`${pname}-deps`): builds *only* the dependency graph. It runs against a synthesized "dummy" source tree (see §4) built from stub `build.rs`/`lib.rs` files and trimmed `Cargo.toml`s (binaries/examples/tests/benches stripped via `fixupCargoToml`) — real application code is never compiled here, only its dependency closure. `copyBins = false`; instead `copyTarget = true` and (by default) `compressTarget = true`, so the output is a `target.tar.zst` containing the compiled dependency artifacts.
2. **`buildTopLevel`** (the actual `pname`): builds the real `src`, but its `configurePhase` first unpacks `buildDeps`' `target.tar.zst` (or rsyncs a raw `target/` dir) into its own `target/` before invoking `cargo build`, so Cargo sees pre-built `.rlib`s for every dependency and only compiles the application's own crates:

```bash
for dep in $builtDependencies; do
    log "pre-installing dep $dep"
    if [ -d "$dep/target" ]; then
      rsync -rl --no-perms --no-owner --no-group --chmod=+w --executability $dep/target/ target
    fi
    if [ -f "$dep/target.tar.zst" ]; then
      zstd -d "$dep/target.tar.zst" --stdout | tar -x
    fi
done
```

This is naersk's **incremental/caching mechanism**: not per-crate derivations, but a two-stage split where the expensive, rarely-changing "compile every dependency" step is cached as its own Nix derivation (content-addressed by everything that affects it — `Cargo.lock`, `Cargo.toml`, the pinned toolchain), and only invalidates when the dependency graph itself changes. Editing your own application source normally only reruns `buildTopLevel`.

- `singleStep = true` disables this split entirely (one derivation does everything) — useful when you need `overrideAttrs` on the final derivation, since overriding only ever affects the top-level drv and would otherwise leave the separately-built deps drv unaffected/inconsistent (README, "Note on `overrideAttrs`").
- Final derivation outputs: `$out/bin` (`copyBins`, default `true`, driven by parsing `cargo build`'s `--message-format=json` stream via `jq`), optionally `$out/lib` (`copyLibs`), optionally a separate `doc` output (`doDoc` + `copyDocsToSeparateOutput`).
- No `Cargo.nix`, no per-crate `.nix` file, nothing checked into version control by naersk itself — the only Nix code is what the caller writes in `flake.nix`/`default.nix` calling `buildPackage { ... }`.

## 3. Hash / fetch story

Naersk does **not** pre-generate per-crate fetcher derivations the way crate2nix does, and it does **not** wrap `cargo fetch`/vendoring in one big fixed-output derivation either. Instead it does something more granular and cheaper than either: **it reads a checksum straight out of `Cargo.lock` for every crates.io crate and does one plain `fetchurl` fixed-output derivation per crate.**

Cargo itself already records, per package, a `checksum = "..."` field in `Cargo.lock` for every registry (crates.io) dependency (in the modern per-package form) or a `[metadata]` table keyed as `"checksum <name> <version> (registry+https://github.com/rust-lang/crates.io-index)"` in the older lockfile format. `lib.nix`'s `mkVersions` reads whichever form is present:

```nix
mkVersions = cargolock:
  if builtins.hasAttr "metadata" cargolock then
    # older format: look up "checksum <name> <version> (registry+...)" in [metadata]
    ...
  else if builtins.hasAttr "package" cargolock then
    # newer format: checksum is a field directly on each [[package]]
    map (p: { inherit (p) name version; sha256 = p.checksum; })
      (builtins.filter (builtins.hasAttr "checksum") cargolock.package)
  else [];
```

Each `{ name, version, sha256 }` becomes its own fixed-output derivation in `build.nix`:

```nix
unpackCratesIoDependency = { name, version, sha256 }:
  let
    crate = fetchurl {
      inherit sha256;
      url = "${cratesDownloadUrl}/${name}/${version}/download";
      name = "download-${name}-${version}";
    };
  in
  runCommandLocal "unpack-${name}-${version}" { }
    ''
      mkdir -p $out
      tar -xzf ${crate} -C $out
      ...
      echo '{"package":"${sha256}","files":{}}' > "$dest/.cargo-checksum.json"
    '';
```

All of these per-crate unpack derivations are symlink-joined (`symlinkJoinPassViaFile`, a closure-list-length-safe reimplementation of `pkgs.symlinkJoin`) into one `unpackedCratesIoDependencies` tree, which is wired into a generated Cargo config as a `[source.crates-io] replace-with` **directory source**:

```nix
cargoconfig = builtinz.writeTOML "config" {
  source = {
    crates-io = { directory = unpackedCratesIoDependencies; };
    git       = { directory = unpackedGitDependencies; };
  } // ...
};
```

That config is copied to `$CARGO_HOME/config.toml` at `configurePhase`, so `cargo build` resolves every crates.io/git dependency from the local Nix store instead of the network — this is what makes the build sandboxable/offline, using Cargo's own [source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html) mechanism rather than `cargo vendor`.

**Git dependencies** (no crates.io checksum exists for these) are handled differently: `lib.nix`'s `findGitDependencies` parses each `git+URL?rev=SHA` / `?tag=` / `?branch=` source string out of `Cargo.lock` and fetches it with Nix's **builtin `fetchGit`**, keyed by the pinned revision (or ref, with `gitAllRefs`/`gitSubmodules` toggles for cases where the pinned commit isn't on the default branch or needs submodules):

```nix
mkFetch = lock: {
  checkout = builtins.fetchGit ({
    url = lock.url;
    rev = lock.revision;
  } // lib.optionalAttrs (lock ? branch) { ref = "refs/heads/${lock.branch}"; }
    // ... );
} // lock;
```

`builtins.fetchGit` is itself content-addressed by the git revision, so no separate Nix-level hash is needed for git deps — the commit SHA already pinned in `Cargo.lock` *is* the integrity guarantee, mirrored by Cargo's own git-dependency locking. A shell script (`unpackGitDependency`, using `cargo metadata` when possible, falling back to grepping `Cargo.toml` files) then locates the right sub-crate inside the checkout, since a git repo may contain a workspace with several crates.

**Net effect: no single opaque "vendor blob" hash to keep in sync — the fetch story is decomposed into one small fixed-output derivation per crate/dependency, each hash sourced directly from data Cargo already put in `Cargo.lock`.** Nothing is prefetched or computed by naersk itself; if `Cargo.lock`'s checksum is wrong or missing, the `fetchurl` simply fails to verify (standard Nix FOD hash-mismatch behavior).

## 4. Dummy source generation (how the deps-only build compiles without app code)

`lib.nix`'s `dummySrc` builds a synthetic project tree used only for the `buildDeps` derivation: it strips `[bin]`, `[[example]]`, `[lib]`, `[[test]]`, `[[bench]]`, `default-run`, and `package.metabuild` from every workspace member's `Cargo.toml` (via `fixupCargoToml`), then writes stub files:

```
echo '//! stub crate'   > build.rs
echo '/// stub main function' >> build.rs
echo 'fn main() {}' >> build.rs

echo '//! stub lib' >src/lib.rs
echo '#![no_std]' >>src/lib.rs
```

A stub `build.rs` is always present (regardless of whether the real crate has one) specifically so Cargo still resolves and builds `[build-dependencies]`; the real `Cargo.toml`'s `build` field is stripped so cargo doesn't try to run any real build script logic in this stage. `#![no_std]` on the stub lib avoids accidentally pulling in `std`'s own compile cost for crates that don't need it. This is a fairly delicate hack, and the issue tracker shows real friction points from it (see §6).

## 5. Why naersk exists — and why choose it over crate2nix

**The gap it fills**: before tools like this, packaging a Rust project for Nix meant either (a) not sandboxing `cargo build`'s network access at all (impure, non-reproducible), or (b) hand-writing/generating a full Nix derivation graph mirroring Cargo's own dependency graph. crate2nix (and originally `carnix`) took path (b): run `crate2nix generate` to produce a checked-in `Cargo.nix` containing one derivation per crate, each independently fetchable/cacheable/buildable, mirroring `buildRustCrate`/`buildGoModule`-style ecosystems in Nix.

naersk instead asks: *what's the least Nix code needed to make `cargo build` itself sandboxable?* Its answer is: intercept dependency fetching via Cargo's own `[source] replace-with` config (§3), and get just enough caching by splitting one build into "deps" + "app" (§2) — no codegen step, no generated file to keep in sync, no `crate2nix generate` re-run required after every `cargo add`/`cargo update`.

**Choose naersk when:**
- You want zero generated/checked-in Nix artifacts — `Cargo.lock` alone drives the build, so there's nothing to regenerate or go stale.
- Your dependency graph isn't enormous, or you don't need cache-hits on individual unchanged dependencies across incremental `cargo update`s (only "did the whole dep graph change or not" granularity).
- You want plain `stdenv.mkDerivation`-shaped output, easy to reason about and override (with caveats, §2/§6).

**Choose crate2nix when:**
- You have a very large or fast-moving dependency tree and want per-crate binary-cache hits, so bumping one leaf dependency doesn't force recompiling every other dependency that happens to share the "deps" derivation in naersk's model.
- You're comfortable running a `generate` step (or accepting IFD) and committing its output, and want dependency-level `overrideAttrs`/patching granularity that naersk's model doesn't offer without disabling its main optimization (`singleStep = true`, see §2).
- (Per secondary sources, unconfirmed against crate2nix's own docs by this document) crate2nix's fine-grained derivation graph reportedly also makes it easier to build/patch individual troublesome dependencies (e.g. ones needing special native `buildInputs`) than naersk's one-big-deps-derivation model, though naersk partially compensates with `crate_specific.nix` (§6) auto-overrides for a curated list of known-troublesome crates (e.g. `openssl-sys` pulling in `pkg-config`/`openssl`).

**Tradeoff table:**

| | naersk | crate2nix |
|---|---|---|
| Input | `Cargo.lock` read directly, every eval | `Cargo.lock` + a generated `Cargo.nix` (checked in or IFD) |
| Generated/checked-in Nix file | none | `Cargo.nix` (or IFD equivalent) |
| Derivation granularity | ~2 derivations (deps, app) + 1 fixed-output fetch per crate | 1 derivation per crate |
| Rebuild on dependency bump | whole "deps" derivation invalidates and rebuilds | only the changed crate (+ its reverse-dependents) rebuilds |
| Cache-sharing across projects | only at the fetch layer (per-crate `fetchurl`), not the compiled-artifact layer | per-crate compiled artifacts are independently cacheable |
| Setup/maintenance burden | call `buildPackage { src = ./.; }`, done | run `crate2nix generate` (or wire up IFD) whenever `Cargo.lock` changes |
| `overrideAttrs` ergonomics | discouraged on the top-level drv unless `singleStep = true` (README) | per-crate overrides are naturally more granular |
| IFD | none by default (`builtins.fromTOML`) | typically none if `Cargo.nix` is generated ahead of time and committed; IFD-based flows exist too |

## 6. Limitations / gotchas

- **Maintenance**: see §0 — formally maintainer-seeking since Dec 2023, though not dead; still gets occasional fixes.
- **`overrideAttrs` footgun**: calling `.overrideAttrs` on a `buildPackage` result only patches the final "app" derivation, not the separately-built "deps" derivation — README explicitly calls this out and recommends passing options directly into `buildPackage`, using its `override`/`overrideMain` hooks, or falling back to `singleStep = true` to disable the two-stage split entirely.
- **`rust-toolchain` file ignored by default** — naersk uses whatever `rustc`/`cargo` nixpkgs provides unless you explicitly wire up `nixpkgs-mozilla`/`rust-overlay` and pass `cargo`/`rustc` into the `pkgs.callPackage naersk { ... }` call.
- **Dummy-source generation is a real source of friction** (from the issue tracker, `nix-community/naersk/issues`, titles as of 2026-07-26): `unused_crate_dependencies fails in cargo workspace unless singleStep is used`; `Can't build a package with submodules in dependencies`; `mode = "test" runs cargo test on dummySrc in deps stage, making two-stage caching ineffective`; `[lib] is not copied over in Cargo.toml`; `No access to root crate file in sub-crate`; `error: failed to parse lock file at: /build/dummy-src/Cargo.lock`. These cluster around workspace edge cases and the stub-file substitution in `dummySrc` (§4) not perfectly mimicking real project shape.
- **Git-dependency friction**: `Fails to fetch cargo dependency using a git repo, maybe because of master vs main`; `Error when using a git dependency that includes a workspace, when the flake is also in a workspace` — the `unpackGitDependency` shell logic (§3) that locates the right sub-crate inside a git checkout is comparatively fragile string/TOML parsing in bash.
- **Cross-compilation** has open friction: `cross-compiling to aarch64-unkown-linux-{gnu,musl} from x86_64-linux` is an open issue; no confirmed first-class story beyond what nixpkgs' cross toolchains + naersk's passthrough options provide.
- **Per-crate auto overrides exist but are a short curated list**: `crate_specific.nix` (16 lines) provides `autoCrateSpecificOverrides` (default `true`) for a small number of known-troublesome crates (e.g. auto-adding `pkg-config`/`openssl` `buildInputs` for `openssl-sys`), not a general solution — anything not on that list needs manual `buildInputs`/`nativeBuildInputs` passed into `buildPackage`.
- **`Cargo.lock` is mandatory and must be tracked/staged** — naersk throws immediately if `Cargo.lock` is missing from `root`, explicitly warning about `.gitignore`d lockfiles or files not staged when using flakes' git-tracked-files-only source filtering.
- **Closure-size gotchas** are opt-out, not opt-in by default in every direction: `remapPathPrefix` (default `true`) and `removeReferencesToSrcFromDocs` (default `true`) both exist specifically to stop `/nix/store` paths to fetched crate sources or generated docs from leaking into the runtime closure — worth being aware of if `gossamer2nix` cares about minimizing FFI-crate closure size for `[rust-bindings] Prebuilt`/`Crates` variants.

## 7. Relevance to `gossamer2nix`'s `[rust-bindings]` handling

Per `docs/DEPS.md` §3.1, `[rust-bindings]` has five variants (`Path`, `Git`, `Crates`, `Src`, `Prebuilt`) that scaffold into a normal Cargo crate resolved by Cargo's own tooling, entirely separate from Gossamer's own `.gos` dependency resolver/lockfile pipeline. That maps fairly directly onto naersk's model:

- A `Crates`/`Git`/`Path` `RustBindingSpec` is, once scaffolded into a `Cargo.toml`, exactly the kind of thing `naersk.buildPackage` (or its `lib.nix` helpers directly) already knows how to turn into a sandboxed, `Cargo.lock`-driven derivation — **no separate `gossamer2nix`-maintained fetcher/hash database needed for the Cargo side**, since naersk (like the Gossamer registry's own `sha256` tarball digests, per `DEPS.md` §7/§9) leans on checksums already present in the relevant lockfile.
- The `Src` variant (a single-file binding with an inlined raw Cargo-deps fragment) and `Prebuilt` variant (archive keyed by ABI version) don't map onto naersk directly — those would need `gossamer2nix`-side scaffolding to synthesize a throwaway `Cargo.toml`/`Cargo.lock` (for `Src`) or bypass Cargo entirely (for `Prebuilt`, which is just a fixed-output-derivation-style archive fetch, no Cargo resolution involved at all).
- naersk's zero-codegen, read-`Cargo.lock`-directly approach is attractive for `gossamer2nix` specifically *because* Gossamer's own `[dependencies]` pipeline (§6–§9 of `DEPS.md`) already generates and enforces its own `project.lock`; adding a second generated-and-must-stay-in-sync file (a crate2nix-style `Cargo.nix`) for the FFI side would be an awkward second lockfile-drift surface. Mirroring Gossamer's "lockfile is the single source of truth" philosophy with naersk's "Cargo.lock is the single source of truth" philosophy keeps both halves of a `gossamer2nix` build conceptually consistent.
- The corresponding weakness also carries over: naersk's per-build "did the whole Cargo dep graph change" cache granularity (§2) means any `[rust-bindings]` change invalidates the *entire* FFI dependency build, not just the changed crate — likely an acceptable tradeoff given FFI dependency graphs are typically far smaller than a full Rust application's, but worth re-checking if a real Gossamer project ends up with a large `[rust-bindings]` closure.
- Given §0's maintenance caveat and secondary sources pointing at `crane` as the community's current default, a `gossamer2nix` design should treat naersk as "known-good, simple, currently working prior art" rather than a long-term dependency to build directly on top of without also evaluating `crane`.

---

## Sources

- [nix-community/naersk](https://github.com/nix-community/naersk), `master` branch, commit `9aa07bb0` (2026-06-23), read via GitHub raw content 2026-07-26: `README.md`, `default.nix`, `lib.nix`, `build.nix`, `config.nix`, `builtins/default.nix`.
- `gh api repos/nix-community/naersk` (metadata: stars/forks/archived/license), `gh api repos/nix-community/naersk/commits`, `gh api repos/nix-community/naersk/issues` (titles, as of 2026-07-26), `gh api repos/nix-community/naersk/issues/320` (maintainer issue body/comments), `gh api repos/nix-community/naersk/pulls/397`.
- Secondary/community sources (flagged unconfirmed where used): a 2025 devenv.sh blog post ("Closing the Nix Gap: From Environments to Packaged Applications for Rust") and a community "Bleeding-edge Rust on NixOS" practitioner's guide, both describing `crane` as naersk's community-perceived successor.
- This repo's `docs/DEPS.md` (read in full as a template for structure/rigor and for the `[rust-bindings]` §3.1 details this document cross-references).

**Re-verify §0 (maintenance status) and the crate2nix comparison in §5 against current upstream before treating them as settled** — maintenance status in particular can change quickly, and the crate2nix side of the comparison here is inferred from naersk's own documented behavior rather than from a direct reading of crate2nix's source (that reading was delegated to a parallel research effort against `docs/deps/CRATE2NIX.md`, not verified by this document).
