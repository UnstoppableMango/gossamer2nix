# Dependency Management in Gossamer

Gossamer has no first-party documentation page for dependency management: [gossamer-lang.org](https://gossamer-lang.org/) and `docs_src/toolchain.md` list `gos` subcommands without explaining semantics.
This document was compiled by reading `SPEC.md` (§6, §16) and the `gossamer-pkg`, `gossamer-resolve`, `gossamer-cli`, and `gossamer-driver` crate source in [danpozmanter/gossamer](https://github.com/danpozmanter/gossamer) (`main` branch, as of 2026-07-25).
It exists to ground the design of this repo's `gossamer2nix` deps-lock/Nix adapter (see [GOALS.md](../GOALS.md)) in the real implementation rather than assumptions.

**This reflects one point-in-time reading of upstream `main` and may drift.**
Re-verify struct/field names and CLI flags against current upstream source before depending on exact details in code.

**Key finding (§12): as read, `gos build` only wires `path`-kind dependencies into compilation; no code path was found consuming a `vendor/` dir or the package cache for registry/git/tarball deps during `build`/`run`/`check`/`test`.
Confirm this against upstream before assuming `gos build --locked` can build a project with non-path dependencies end-to-end.**

---

## 1. Project manifest (`project.toml`)

Every Gossamer project is defined by a `project.toml` manifest at its root, "the unit of distribution, versioning, and dependency declaration" (SPEC §6.4).

```toml
[project]
id      = "example.com/math"
version = "0.3.1"
edition = "2026"
authors = ["Jane Doe <jane@example.com>"]
license = "Apache-2.0"

[dependencies]
"example.org/linalg"   = "1.2"
"example.com/logging"  = { git = "https://git.example.com/logging.git", tag = "v0.8.0" }
"example.net/internal" = { path = "../internal" }

[registries]
"example.org" = "https://registry.example.org/v1"

[trusted-publishers]
"example.org/linalg" = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[rust-bindings]
# see §3.1

[[bin]]
name = "mathctl"
# path defaults to src/bin/mathctl.gos

[lib]
# name defaults to project id's leaf; path defaults to src/lib.gos
```

### `[project]` fields

| Field | Type | Required | Default |
|---|---|---|---|
| `id` | `ProjectId` (§2) | yes | none |
| `version` | SemVer 2.0.0 (§5) | yes | none |
| `edition` | `"2026"` \| `"2027"` | no | `"2026"` |
| `authors` | array of strings | no | `[]` |
| `license` | string | no | `""` |
| `output` | string | no | binary artifact filename override |
| `entry` | string | no | explicit entry source path override |

### Other top-level sections

| Section | Type | Purpose |
|---|---|---|
| `[dependencies]` | table | maps a `ProjectId` string to a `DependencySpec` (§3) |
| `[registries]` | table | maps a DNS prefix to a registry base URL |
| `[trusted-publishers]` | table | maps a project id to a 64-hex-char Ed25519 public key |
| `[rust-bindings]` | table | maps a Cargo crate name to a `RustBindingSpec` (§3.1, FFI) |
| `[[bin]]` | array of tables | explicit binary targets |
| `[lib]` | table | optional library target |

### Validation rules

- `project.id` must parse via the `ProjectId` grammar (§2); `project.version` must parse as strict SemVer 2.0.0.
- `edition` accepts only `"2026"` or `"2027"`.
- `[trusted-publishers]` values must be exactly 64 hex characters, normalized to lowercase.
- `[rust-bindings]` keys must match `[A-Za-z_][A-Za-z0-9_-]*`.
- Git dependency URLs must be absolute HTTPS or SSH with a valid host.
- Registry URLs must be absolute HTTP(S) with a valid host.
- Tarball dependencies (`{ url, sha256 }`) require the `sha256` field: there is no unchecksummed URL dependency.
- `[[bin]]` names must be unique.
- **Manifest size is capped at 1 MB.**

### Defaults for targets

- `bin[].path` → `src/bin/<name>.gos`
- `lib.name` → the project id's leaf path segment
- `lib.path` → `src/lib.gos`
- a git dependency's ref → `"main"` if none of `tag`/`branch`/`rev` given
- `rust-bindings` `default-features` → `true`
- `rust-bindings` prebuilt ABI → `"1.0"`

---

## 2. Project identifiers (`ProjectId`)

Grammar (SPEC §6.5):

```
ProjectId     = DomainSegment { "/" PathSegment }
DomainSegment = Label { "." Label }        // must contain at least one "."
Label         = [a-z][a-z0-9-]*
PathSegment   = [a-z0-9][a-z0-9-_]*
```

Examples: `example.com/math`, `acme.dev/tools/codegen`, `fooware.io/json`.

Path segments may contain `_` but only mid-segment; a leading `_` is rejected, deliberately, so package names can't collide visually with Rust's unused-binding convention.

Key properties (SPEC + `gossamer-pkg/src/id.rs`):

- **Not a URL.**
  It names no server, repository, or protocol.
- **Not tied to a hosting provider**: use a domain you control.
- **Ownership is social, not technical**: no global authority enforces who may publish under a given id.
- **Single-segment bare ids (`math`, `fmt`) are reserved for the standard library.**

`ProjectId::domain()` returns the DNS-prefix portion (used to match `[registries]` entries), `path()` returns everything after the first `/`, and `tail()` returns the last path segment (or the domain if there is no path), used by the compiler as the default `use` binding name.

---

## 3. Dependency source kinds

A `[dependencies]` entry (`DependencySpec`) is either a bare version literal (registry, implicitly) or an inline table selecting one of three other source kinds:

| Kind | TOML shape | Notes |
|---|---|---|
| Registry | `"example.org/linalg" = "1.2"` | caret-range version (§5); resolved against `[registries]` |
| Git | `{ git = "https://...", tag = "..." }` \| `{ branch = "..." }` \| `{ rev = "..." }` | ref defaults to `"main"` |
| Local path | `{ path = "../internal" }` | side-by-side development, no fetch/lock digest |
| URL tarball | `{ url = "https://...", sha256 = "..." }` | `sha256` is mandatory |

Only one inline kind may be given per dependency (git/path/tarball are mutually exclusive; you cannot mix `branch`/`tag`/`rev` within a git entry).

### 3.1 `[rust-bindings]` (FFI, not `.gos` deps)

Separate from `[dependencies]`, but declared in the same manifest, and worth documenting because a nix adapter for a real project will very likely need to build these too.
Five `RustBindingSpec` variants:

- `Path { version?, path, features, default_features }`: local Cargo crate
- `Git { version?, url, reference?, features, default_features }`: Cargo crate from a git repo
- `Crates { version, features, default_features }`: from crates.io
- `Src { src, deps }`: single-file binding with a raw Cargo-deps fragment inlined in the manifest
- `Prebuilt { archive, abi }`: pre-built static archive keyed by ABI version

This whole FFI path is orthogonal to registry/git/tarball `.gos` dependency resolution described below, and uses Cargo's own resolution once scaffolded, so it's a separate dependency graph a nix adapter needs to handle via normal Cargo/crates.io tooling, not this pipeline.

---

## 4. Module system and `use` declarations

Modules are directory-based (SPEC §6.3).
Given:

```
my-project/
  src/
    main.gos
    math.gos
    math/
      vector.gos
      matrix.gos
```

- Every `.gos` file directly in `src/` contributes to the project's root module.
- Each subdirectory of `src/` is a module named after the directory.
- Modules nest: `src/math/vector.gos` is `math::vector`.
- Inline modules are also supported: `mod vector { ... }`.

Import syntax (SPEC §6.6) covers both same-project and cross-project paths:

```
use "example.com/math"                  // binds `math` (external project, by id)
use "example.com/math" as m             // renamed binding
use "example.com/math"::vector          // reach into a specific module of an external project
use vector::{Vec3, Vec4}                // same-project path
use std::io                             // standard library, always requires an explicit use
```

Writing a qualified path like `fs::read(path)` without `use std::fs` first is an unresolved-name error; there's no implicit prelude beyond a handful of always-in-scope macros (e.g. `println!`).

An external `use "id"` string is exactly a `[dependencies]` key, and that's the sole link between the manifest and the source-level import graph.

---

## 5. Version semantics

`Version` (`gossamer-pkg/src/version.rs`) is strict SemVer 2.0.0: numeric segments reject leading zeros, prerelease identifiers are dot-separated alphanumeric/hyphen, build metadata is retained for display/lockfile fidelity but never affects precedence.

Prerelease ordering: numeric identifier segments compare as integers, numeric ranks below non-numeric, non-numeric compares lexicographically; a version *without* a prerelease outranks one *with* a prerelease at the same base tuple.
This matters for resolution: "a registry cannot accidentally resolve `1.0.0-alpha` as the final `1.0.0`."

**Caret ranges** (`^x.y.z`, the only range syntax; a bare version literal in `[dependencies]` is sugar for a caret range):

- For `x ≥ 1`: matches `[x.y.z, (x+1).0.0)`, pinning the major.
- For `0.x.y`: matches `[0.x.y, 0.(x+1).0)`, pinning the minor (Cargo-style 0.x semantics).
- Prereleases are excluded from a normal caret requirement unless the minimum itself names a prerelease (`^1.2.0-rc.1` matches `1.2.0-rc.2` but not `1.3.0-rc.1`; prerelease matching stays within the base tuple).

---

## 6. Dependency resolution algorithm

Implemented in `gossamer-pkg/src/resolver.rs`.
**This is not a SAT/PubGrub solver: it's greedy, single-pass, non-backtracking:**

1. Parse and normalize: convert each `DependencySpec` into a `RequirementSpec` (caret range, or an inline pin).
2. **Aggregate requirements**: collect every range/pin declared for a given project id across all consumers currently in the graph.
3. **Inline conflicts fail immediately**: if two consumers pin the same project id to different git/path/tarball sources, that's a hard error (`ConflictingPins`); inline and registry pins for the same id cannot coexist either.
4. **Resolve registry deps**: for each id, iterate candidate versions in *descending* order and take the first version where every collected range matches it (`pick_highest`), i.e. intersection is done by predicate testing, not by computing an explicit intersection.
   - Yanked versions are skipped unconditionally during selection.
   - No candidate satisfying all ranges → `Unsatisfiable`.
   - Ranges with an empty intersection → `IncompatibleVersions`, with a `detail` field listing tried versions and requirements for diagnostics.
5. **Recurse transitively**: `resolve_transitive` runs a breadth-first work-queue walk, where each selected dependency's `project.toml` is loaded (via a `TransitiveLoader` trait implementation) and its own dependencies are pushed onto the queue.
   Cycles are prevented with a visited set keyed on `{raw_id}|{debug_pin}`.
6. **Sort output** lexicographically by project id for determinism.

Known limitation worth flagging for a Nix adapter: **there is no backtracking**.
If the highest version picked for dependency A turns out to require a version of B that's incompatible with some other constraint on B discovered later, the resolver does not retry with a lower A; this is a "first-fit" resolver, not a constraint solver.
A `gossamer2nix` deps-lock generator that wants to reproduce `gos`'s actual resolution decisions should mirror this exact greedy-descending algorithm rather than run a more sophisticated solver that might pick a different (and disagreeing) graph.

---

## 7. Lockfile (`project.lock`)

Format (`gossamer-pkg/src/lockfile.rs`), written by `gos fetch`/`gos update`:

```toml
# gossamer project.lock v1

[[project]]
id     = "example.org/linalg"
source = "registry"
version = "1.2.4"
sha256 = "…"                # source-tree digest, hex
owner_pubkey = "…"          # publisher ed25519 key, hex, registry only

[[project]]
id     = "example.com/logging"
source = "git"
url    = "https://git.example.com/logging.git"
ref    = "v0.8.0"
sha256 = "…"

[[project]]
id     = "example.net/internal"
source = "path"
path   = "../internal"
# no sha256: path sources are never digest-pinned
```

- The leading `# gossamer project.lock v1` line is a magic/version header for sanity-checking the lockfile format itself.
- Entries are sorted deterministically by project id string.
- Per source kind, the recorded fields differ: registry uses `version`; git uses `url` + `ref`; path uses `path`; tarball uses `url` + `tarball_sha256`.
- `sha256` is the content-addressed digest of the *fetched and verified source tree* (see §9), omitted for path sources, which aren't fetched or cached.
- `owner_pubkey` is recorded only for registry sources whose tarball carried a valid signature.
- Lockfile errors: `Malformed`, `MissingField`, `MissingPin` (resolver found a dependency with no matching lock entry), `Drift` (resolved vs. locked pin/digest mismatch), `LockfileMissing`.

`--locked` (available on `build`/`run`/`watch`) requires `project.lock` to be present and to exactly match what the resolver would currently produce for every dependency; this is the reproducibility guarantee, and the natural thing for a Nix build to assert/rely on.
`check` and `test` do **not** enforce lockfile consistency.

Registry-published tarballs are packed deterministically (fixed file order, zeroed timestamps/ownership, normalized file mode), which is what makes a registry dependency's `sha256` pin stable and reproducible across machines and producers, directly relevant to building a Nix fixed-output derivation around a fetched package.

---

## 8. Fetching and verification

`gossamer-pkg/src/fetch.rs` dispatches per source kind:

- **Path**: read directly from the local filesystem, no fetch/cache/digest involved.
- **Registry**: looks up the requested version in the registry's version catalogue (§10) for a download URL and tarball checksum; rejects yanked versions unless explicitly overridden; requires a valid Ed25519 signature over the tarball.
- **Git**: clones the repo and extracts the requested ref into a tarball.
  Refs used for locked/pinned fetches must be full 40- or 64-hex-char object ids, not branch/tag names, so a pin always resolves to one immutable commit.
- **Tarball**: downloaded over HTTP(S), verified against the declared `sha256`, optionally signature-checked, then unpacked.

Verification (digest, signature, key-pinning against either an existing `project.lock` pin or a manifest `[trusted-publishers]` entry) always happens **before** the fetched content is unpacked or cached, so a corrupted or mismatched download never reaches disk as usable source.

`gos vendor` writes fetched sources out to a directory (default `vendor/`) for offline builds, guarding against path traversal from malicious archive entries during materialization.

---

## 9. Local cache

`gossamer-pkg/src/cache.rs`:

- Cache root: `$GOS_CACHE_DIR` if set, else `$HOME/.gossamer/cache`.
- Layout: every cached, verified source tree lives at `<cache-root>/pkg/<sha256>/source/`, paired with an `id.txt` sidecar file so the runtime can re-hash on read and reject silent corruption.
- **Cache key** = sha256 of the canonical serialization of the fetched file map (content-addressed, not source-kind- or version-keyed), the same digest that ends up in `project.lock`.
- Admission limits per package: 4096 files, 16 MiB per file, 64 MiB aggregate.
- **No eviction, pruning, or TTL logic exists in this module**: the cache only validates and stores; nothing in `gossamer-pkg` ever shrinks it.
  (`gos cache --prune` exists at the CLI layer per `docs_src/toolchain.md`, removing files older than 30 days or over a configured cap; that policy lives in the CLI/driver layer, not in `gossamer-pkg::cache`.)

---

## 10. Registry protocol

A registry is a plain HTTP(S) service; nothing about it is gossamer-specific infrastructure.
`[registries]` maps a DNS-style domain prefix to a base URL; a dependency's registry is selected by matching its project id's domain against that table (exact matching-precedence logic wasn't found in the excerpted source; treat as **unconfirmed** rather than assumed longest-prefix matching).

Per SPEC §16.8, a registry "maps `/v1/<project-id>/<version>` to a signed tarball plus metadata."
The per-project version catalogue consulted during resolution (§6) holds, per version: `version`, `yanked`, `yank_reason`, `download_url`, `tarball_sha256`, `signature`, `public_key`.
**The exact HTTP GET endpoint used to build that catalogue (the index/version-listing call) was not located in the crate excerpts read for this document**; do not assume a specific path format for it without re-checking source.

Registries are optional and federated: "no central registry is shipped with the toolchain and none is required to use Gossamer" (SPEC §16.8).
A registry tarball must carry a valid Ed25519 signature whose advertised publisher key matches either an existing `project.lock` pin or a `[trusted-publishers]` binding; this is the entire trust model, there's no CA-like central authority.

Private registries authenticate reads with a bearer token from `~/.gossamer/credentials.toml` (managed by `gos login`/`gos logout`).
Publishing (`gos publish`/`gos yank`/`gos owner`) has its own upload protocol, but that's a producer-side concern, not part of resolving or building against dependencies, so it's out of scope here.

---

## 11. CLI command reference (dependency-relevant subset)

| Command | Flags | Effect |
|---|---|---|
| `gos add SPEC` | `--manifest PATH`, `--rust-binding` | adds a `[dependencies]` (or `[rust-bindings]`) entry; `SPEC` = `id`, `id@version`, or (for rust-binding) `crate`/`crate@version`/`path:dir` |
| `gos remove ID` | `--manifest PATH` | drops a dependency entry, errors if absent |
| `gos tidy` | `--manifest PATH` | drops unused direct deps, canonicalizes manifest ordering |
| `gos update` | none | re-walks the registry index and re-resolves, ignoring the existing lock (delegates to `fetch(manifest, offline=false, update=true)`) |
| `gos fetch` | `--manifest PATH`, `--offline`, `--update` | resolves + populates the local cache + writes/refreshes `project.lock`; `--offline` refuses to populate any cache entry not already present |
| `gos vendor` | `--manifest PATH`, `--out DIR` (default `vendor`) | materializes all transitive deps to disk for offline builds |
| `gos build`/`gos run`/`gos watch` | `--locked` | requires `project.lock` present and matching resolver output for every dependency |
| `gos check`/`gos test` | none | **no** lockfile enforcement |

(`gos publish`, `gos yank`, `gos login`/`gos logout`, `gos owner` also exist, for the registry-publishing workflow; irrelevant to building/consuming dependencies, so omitted here.)

`gos build` itself also takes `--release`, `-g`, `--dynamic`, `--target TRIPLE`, `--out-dir PATH`, and PGO flags (`--pgo-collect`, `--pgo-profile`), unrelated to dependency handling but relevant to a Nix builder wrapping it (this repo's `nix/builder.nix` already passes these through as `gosBuildFlags`).

---

## 12. How `gos build` actually consumes dependencies: a gap to design around

This is the most load-bearing finding for the Nix adapter, verified by reading `gossamer-cli/src/paths.rs`, `cmd/build.rs`, `cmd/check.rs`, `main.rs`, `cli.rs`, and driver `frontend.rs`/`pipeline.rs`:

**Only `path`-kind dependencies are wired into compilation**, and even those are handled by naive source concatenation, not real multi-crate linking.
`bundle_path_dependencies()` (`paths.rs`) walks path-typed `DependencySpec::Inline(InlineDependency::Path)` entries recursively and literally splices each dependency's bundled source into the compiled buffer as an inline module:

```rust
out.push_str(&format!(
    "\n// auto-bundled dependency: {} ({})\nmod {} {{\n{}\n}}\n",
    dep_id, dep_root.display(), mod_name, dep_bundled
));
```

**No code path was found** in the CLI (`build.rs`, `check.rs`, `main.rs`, `cli.rs`) or in `gossamer-driver` (`frontend.rs`, `pipeline.rs`) that reads a `vendor/` directory or the global `~/.gossamer/cache` during `build`/`run`/`check`/`test` to pull in registry, git, or tarball dependency source.
`gossamer-resolve/src/external.rs` (which the frontend consults for external module lookups) is purely an in-memory table populated by `set_external_modules` at process startup; its caller/data source was not located in the crates read for this document.

**Practical implication:** as implemented today, `gos fetch` / `gos vendor` / `project.lock` are a fully real fetch-and-lock pipeline, but there is no confirmed mechanism by which their output actually reaches the compiler for non-path dependencies.
Either (a) this wiring exists somewhere not covered by this reading and should be re-checked before finalizing the adapter design, or (b) registry/git/tarball dependencies are not yet consumable by `gos build` at all, and only `path`-style deps are currently real for compilation purposes.
**Treat this as the single highest-priority thing to confirm against upstream (or upstream maintainers) before committing to a `gossamer2nix` design** that assumes `gos build --locked` can build a project with non-path dependencies end-to-end inside a sandboxed derivation.

---

## Sources

- `SPEC.md` §6 (Projects, Modules, `use`, dependency resolution, lockfile, registries) and §16 (toolchain command reference), in [danpozmanter/gossamer](https://github.com/danpozmanter/gossamer).
- `crates/gossamer-pkg/src/{manifest,lockfile,resolver,fetch,cache,version,id}.rs`
- `crates/gossamer-cli/src/{cli.rs,paths.rs,cmd/{pkg,build,check}.rs,main.rs}`
- `crates/gossamer-resolve/src/external.rs`
- `crates/gossamer-driver/src/{frontend,pipeline}.rs`

Read directly from upstream `main` on 2026-07-25 via GitHub's raw content and contents APIs; no local clone of the gossamer repo exists in this workspace.
Sections 10 and 12 explicitly flag findings that could not be fully confirmed from the excerpts read; re-verify those against current upstream source (or ask upstream maintainers) before relying on them.
