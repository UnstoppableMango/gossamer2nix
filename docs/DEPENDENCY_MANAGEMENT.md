# Dependency Management in Gossamer

Gossamer has no first-party documentation page for dependency management:
[gossamer-lang.org](https://gossamer-lang.org/) and `docs_src/toolchain.md`
list `gos` subcommands without explaining semantics. This document was
compiled by reading `SPEC.md` (§6, §16) and the `gossamer-pkg`,
`gossamer-resolve`, `gossamer-cli`, and `gossamer-driver` crate source in
[danpozmanter/gossamer](https://github.com/danpozmanter/gossamer) (`main`
branch, as of 2026-07-25). It exists to ground the design of this repo's
`gossamer2nix` deps-lock/Nix adapter (see [GOALS.md](../GOALS.md)) in the
real implementation rather than assumptions.

**This reflects one point-in-time reading of upstream `main` and may drift.**
Re-verify struct/field names and CLI flags against current upstream source
before depending on exact details in code.

---

## 1. Project manifest (`project.toml`)

Every Gossamer project is defined by a `project.toml` manifest at its root,
"the unit of distribution, versioning, and dependency declaration" (SPEC §6.4).

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

- `project.id` must parse via the `ProjectId` grammar (§2); `project.version`
  must parse as strict SemVer 2.0.0.
- `edition` accepts only `"2026"` or `"2027"`.
- `[trusted-publishers]` values must be exactly 64 hex characters, normalized
  to lowercase.
- `[rust-bindings]` keys must match `[A-Za-z_][A-Za-z0-9_-]*`.
- Git dependency URLs must be absolute HTTPS or SSH with a valid host.
- Registry URLs must be absolute HTTP(S) with a valid host.
- Tarball dependencies (`{ url, sha256 }`) require the `sha256` field:
  there is no unchecksummed URL dependency.
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

Path segments may contain `_` but only mid-segment; a leading `_` is
rejected, deliberately, so package names can't collide visually with Rust's
unused-binding convention.

Key properties (SPEC + `gossamer-pkg/src/id.rs`):

- **Not a URL.** It names no server, repository, or protocol.
- **Not tied to a hosting provider**: use a domain you control.
- **Ownership is social, not technical**: no global authority enforces who
  may publish under a given id.
- **Single-segment bare ids (`math`, `fmt`) are reserved for the standard
  library.**

`ProjectId::domain()` returns the DNS-prefix portion (used to match
`[registries]` entries), `path()` returns everything after the first `/`,
and `tail()` returns the last path segment (or the domain if there is no
path), used by the compiler as the default `use` binding name.

---

## 3. Dependency source kinds

A `[dependencies]` entry (`DependencySpec`) is either a bare version literal
(registry, implicitly) or an inline table selecting one of three other
source kinds:

| Kind | TOML shape | Notes |
|---|---|---|
| Registry | `"example.org/linalg" = "1.2"` | caret-range version (§5); resolved against `[registries]` |
| Git | `{ git = "https://...", tag = "..." }` \| `{ branch = "..." }` \| `{ rev = "..." }` | ref defaults to `"main"` |
| Local path | `{ path = "../internal" }` | side-by-side development, no fetch/lock digest |
| URL tarball | `{ url = "https://...", sha256 = "..." }` | `sha256` is mandatory |

Only one inline kind may be given per dependency (git/path/tarball are
mutually exclusive; you cannot mix `branch`/`tag`/`rev` within a git entry).

### 3.1 `[rust-bindings]` (FFI, not `.gos` deps)

Separate from `[dependencies]`, but declared in the same manifest, and
worth documenting because a nix adapter for a real project will very likely
need to build these too. Five `RustBindingSpec` variants:

- `Path { version?, path, features, default_features }`: local Cargo crate
- `Git { version?, url, reference?, features, default_features }`: Cargo
  crate from a git repo
- `Crates { version, features, default_features }`: from crates.io
- `Src { src, deps }`: single-file binding with a raw Cargo-deps fragment
  inlined in the manifest
- `Prebuilt { archive, abi }`: pre-built static archive keyed by ABI version

`gos add --rust-binding <spec>` scaffolds a wrapper crate under
`.gos-bindings/<name>/` if the dependency doesn't already provide
`gossamer-binding` entry points (via `register_module!`). This whole FFI
path is orthogonal to registry/git/tarball `.gos` dependency resolution
described below, and uses Cargo's own resolution once scaffolded.

---

## 4. Module system and `use` declarations

Modules are directory-based (SPEC §6.3). Given:

```
my-project/
  src/
    main.gos
    math.gos
    math/
      vector.gos
      matrix.gos
```

- Every `.gos` file directly in `src/` contributes to the project's root
  module.
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

Writing a qualified path like `fs::read(path)` without `use std::fs` first
is an unresolved-name error; there's no implicit prelude beyond a handful
of always-in-scope macros (e.g. `println!`).

An external `use "id"` string is exactly a `[dependencies]` key, and that's
the sole link between the manifest and the source-level import graph.

---

## 5. Version semantics

`Version` (`gossamer-pkg/src/version.rs`) is strict SemVer 2.0.0: numeric
segments reject leading zeros, prerelease identifiers are dot-separated
alphanumeric/hyphen, build metadata is retained for display/lockfile
fidelity but never affects precedence.

Prerelease ordering: numeric identifier segments compare as integers,
numeric ranks below non-numeric, non-numeric compares lexicographically; a
version *without* a prerelease outranks one *with* a prerelease at the same
base tuple. This matters for resolution: "a registry cannot accidentally
resolve `1.0.0-alpha` as the final `1.0.0`."

**Caret ranges** (`^x.y.z`, the only range syntax; a bare version literal
in `[dependencies]` is sugar for a caret range):

- For `x ≥ 1`: matches `[x.y.z, (x+1).0.0)`, pinning the major.
- For `0.x.y`: matches `[0.x.y, 0.(x+1).0)`, pinning the minor (Cargo-style
  0.x semantics).
- Prereleases are excluded from a normal caret requirement unless the
  minimum itself names a prerelease (`^1.2.0-rc.1` matches `1.2.0-rc.2` but
  not `1.3.0-rc.1`; prerelease matching stays within the base tuple).

---

## 6. Dependency resolution algorithm

Implemented in `gossamer-pkg/src/resolver.rs`. **This is not a SAT/PubGrub
solver: it's greedy, single-pass, non-backtracking:**

1. Parse and normalize: convert each `DependencySpec` into a
   `RequirementSpec` (caret range, or an inline pin).
2. **Aggregate requirements**: collect every range/pin declared for a given
   project id across all consumers currently in the graph.
3. **Inline conflicts fail immediately**: if two consumers pin the same
   project id to different git/path/tarball sources, that's a hard error
   (`ConflictingPins`); inline and registry pins for the same id cannot
   coexist either.
4. **Resolve registry deps**: for each id, iterate candidate versions in
   *descending* order and take the first version where every collected
   range matches it (`pick_highest`), i.e. intersection is done by
   predicate testing, not by computing an explicit intersection.
   - Yanked versions are skipped unconditionally during selection.
   - No candidate satisfying all ranges → `Unsatisfiable`.
   - Ranges with an empty intersection → `IncompatibleVersions`, with a
     `detail` field listing tried versions and requirements for
     diagnostics.
5. **Recurse transitively**: `resolve_transitive` runs a breadth-first
   work-queue walk, where each selected dependency's `project.toml` is loaded
   (via a `TransitiveLoader` trait implementation) and its own dependencies
   are pushed onto the queue. Cycles are prevented with a visited set keyed
   on `{raw_id}|{debug_pin}`.
6. **Sort output** lexicographically by project id for determinism.

Known limitation worth flagging for a Nix adapter: **there is no
backtracking**. If the highest version picked for dependency A turns out to
require a version of B that's incompatible with some other constraint on B
discovered later, the resolver does not retry with a lower A; this is a
"first-fit" resolver, not a constraint solver. A `gossamer2nix` deps-lock
generator that wants to reproduce `gos`'s actual resolution decisions
should mirror this exact greedy-descending algorithm rather than run a more
sophisticated solver that might pick a different (and disagreeing) graph.

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

- The leading `# gossamer project.lock v1` line is a magic/version header
  for sanity-checking the lockfile format itself.
- Entries are sorted deterministically by project id string.
- Per source kind, the recorded fields differ: registry uses `version`;
  git uses `url` + `ref`; path uses `path`; tarball uses `url` +
  `tarball_sha256`.
- `sha256` is the content-addressed digest of the *fetched and verified
  source tree* (see §9), omitted for path sources, which aren't fetched or
  cached.
- `owner_pubkey` is recorded only for registry sources whose tarball
  carried a valid signature.
- Lockfile errors: `Malformed`, `MissingField`, `MissingPin` (resolver
  found a dependency with no matching lock entry), `Drift` (resolved vs.
  locked pin/digest mismatch), `LockfileMissing`.

`--locked` (available on `build`/`run`/`watch`) requires `project.lock` to
be present and to exactly match what the resolver would currently produce
for every dependency; this is the reproducibility guarantee, and the
natural thing for a Nix build to assert/rely on. `check` and `test` do
**not** enforce lockfile consistency.

---

## 8. Fetching and verification

`gossamer-pkg/src/fetch.rs`'s `Fetcher::fetch_one` dispatches per source
kind:

- **Path**: walks the local tree recursively, rejects symlinks, enforces
  `MAX_CACHED_SOURCE_FILES` / `MAX_CACHED_SOURCE_BYTES`, and produces an
  in-memory `BTreeMap<String, Vec<u8>>` of relative path → file bytes.
- **Registry**: looks up the requested version in a pre-populated
  `VersionCatalogue` for `download_url` + `tarball_sha256`; rejects yanked
  versions unless `allow_yanked` is set; requires a valid Ed25519 signature
  (`SignatureCheck::verify_reader`).
- **Git**: validates the URL and ref (`validate_git_source`), then shells
  out to a **hardened** `git`:
  - `git clone --bare` into the cache, then `git archive` to extract the
    requested ref into a tarball.
  - Remote helpers, the `file://` transport, and interactive prompts are
    all disabled (`protocol.ext.allow=never`, hardened invocation).
  - Refs accepted for locked/pinned fetches must be full 40- or 64-hex-char
    object ids, which blocks branch/tag ref-confusion/traversal attacks.
- **Tarball**: downloaded via the custom `Transport::get_to_writer`,
  verified against the declared `sha256`, optionally signature-checked,
  then unpacked (`tar::unpack_reader`), bounded by
  `tar::MAX_PACKAGE_ARCHIVE_BYTES`.

Downloads stream to a temporary spool file created with mode `0o600`
(owner-only); a `HashingFile` wrapper computes the sha256 incrementally
while writing. Verification (digest, signature, key-pinning against either
an existing `project.lock` pin or a manifest `[trusted-publishers]` entry)
happens **before** unpacking.

`vendor()` (backing `gos vendor`) writes cached/fetched sources out to disk,
checking every path with `is_safe_package_path()` to reject directory
traversal during materialization.

Errors surface as `CacheError` variants: `DigestMismatch`,
`SignatureInvalid`, `KeyMismatch`, `UntrustedPublisher`, `Yanked`,
`PathUnreadable`, `CacheIo`, `Unsupported`.

---

## 9. Local cache

`gossamer-pkg/src/cache.rs`:

- Cache root: `$GOS_CACHE_DIR` if set, else `$HOME/.gossamer/cache`.
- Layout: every cached, verified source tree lives at
  `<cache-root>/pkg/<sha256>/source/`, paired with an `id.txt` sidecar file
  so the runtime can re-hash on read and reject silent corruption.
- **Cache key** = sha256 of the canonical `path\0bytes\0...` serialization
  of the file map (content-addressed, not source-kind- or version-keyed).
  `SourceTreeDigest` supports incremental hashing (`update_file` in sorted
  path order) so the digest can be computed without a second tree walk.
- Admission limits: `MAX_CACHED_SOURCE_FILES` = 4096,
  `MAX_CACHED_SOURCE_FILE_BYTES` = 16 MiB, `MAX_CACHED_SOURCE_BYTES` = 64
  MiB aggregate, per package.
- **No eviction, pruning, or TTL logic exists in this module**: the cache
  only validates and stores; nothing in `gossamer-pkg` ever shrinks it.
  (`gos cache --prune` exists at the CLI layer per `docs_src/toolchain.md`,
  removing files older than 30 days or over a configured cap; that policy
  lives in the CLI/driver layer, not in `gossamer-pkg::cache`.)

---

## 10. Registry protocol

Custom HTTP(S) client (`gossamer-pkg/src/transport.rs`), no external HTTP
library:

- `std::net::TcpStream` for plain HTTP, `rustls` (Mozilla/`webpki-roots`
  root CAs) for HTTPS, hand-rolled HTTP/1.1 parsing.
- Timeouts: 10s connect, 30s read/write. **No retry logic.**
- **No proxy support**: hostnames are resolved directly.
- Response limits: 64 MiB body, 64 KiB headers.
- HTTP-to-non-loopback-hosts is rejected unless
  `GOS_ALLOW_INSECURE_REGISTRY=1` is set (`new_mozilla_roots_insecure`).

`[registries]` maps a DNS-style domain prefix to a base URL; a dependency's
registry is selected by matching its `ProjectId::domain()` against that
table (exact matching-precedence logic wasn't found in the excerpted
source; treat as **unconfirmed** rather than assumed longest-prefix
matching).

Download side: per SPEC §16.8, a registry is "a plain HTTP service that
maps `/v1/<project-id>/<version>` to a signed tarball plus metadata." The
in-memory `VersionCatalogue`/`CatalogueEntry` (queried during resolution,
§6) is keyed by project id and holds, per version: `version`, `yanked`,
`yank_reason`, `download_url`, `tarball_sha256`, `signature`,
`public_key`. The `Fetcher` is handed an already-populated
`VersionCatalogue` (`with_catalogue`). **The exact HTTP GET endpoint used
to build that catalogue (the index/version-listing call) was not located
in the crate excerpts read for this document**; do not assume a specific
path format for it without re-checking source.

Upload side (`gossamer-pkg/src/publish.rs`), used by `gos publish`/`gos
yank`/`gos owner`:

| Operation | Method | Path | Body |
|---|---|---|---|
| Upload (legacy v1) | PUT/POST | `/v1/upload/{id}/{version}` | JSON: base16 artifact + sha256 + optional signature/pubkey |
| Upload (streaming v2) | PUT/POST | `/v1/upload/{id}/{version}` | raw USTAR bytes; headers `X-Gossamer-Publish-Protocol: 2`, `X-Gossamer-Artifact-Sha256`, `X-Gossamer-Signature-Input`, `X-Gossamer-Signature`, `X-Gossamer-Public-Key` |
| Yank | POST | `/v1/yank/{id}/{version}` | JSON `{ reason? }` |
| Owner | POST | `/v1/owners/{id}` | JSON `{ op: "add"\|"remove"\|"list", user? }` |

Authenticated requests send `Authorization: Bearer <token>`. Registries are
optional and federated: "no central registry is shipped with the
toolchain and none is required to use Gossamer" (SPEC §16.8). A registry
tarball must carry a valid Ed25519 signature whose advertised publisher key
matches either an existing `project.lock` pin or a `[trusted-publishers]`
binding; this is the entire trust model, there's no CA-like central
authority.

### Credentials

`gossamer-pkg/src/credentials.rs`: `$GOS_CREDENTIALS_FILE`, else
`~/.gossamer/credentials.toml`:

```toml
[registries."https://registry.example.org/v1"]
token = "…"
```

Stored in plaintext but file permissions are forced to `0o600` (POSIX) /
owner-only DACL (Windows), written atomically (temp file + rename, with the
permission applied before rename). `gos login --registry URL` writes a
token (interactive prompt, or `$GOS_TOKEN`); `gos logout --registry URL`
removes it.

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
| `gos publish` | `--registry URL`, `--dry-run` | deterministically packs, hashes, optionally signs, uploads |
| `gos yank <id>@<ver>` | `--reason MSG` | marks a published version yanked |
| `gos login`/`gos logout` | `--registry URL` | manage bearer-token credentials |
| `gos owner` | `add\|remove\|list <id> [<user>]` | registry ACL management |
| `gos build`/`gos run`/`gos watch` | `--locked` | requires `project.lock` present and matching resolver output for every dependency |
| `gos check`/`gos test` | none | **no** lockfile enforcement |

`gos build` itself also takes `--release`, `-g`, `--dynamic`, `--target
TRIPLE`, `--out-dir PATH`, and PGO flags (`--pgo-collect`,
`--pgo-profile`), unrelated to dependency handling but relevant to a Nix
builder wrapping it (this repo's `nix/builder.nix` already passes these
through as `gosBuildFlags`).

---

## 12. How `gos build` actually consumes dependencies: a gap to design around

This is the most load-bearing finding for the Nix adapter, verified by
reading `gossamer-cli/src/paths.rs`, `cmd/build.rs`, `cmd/check.rs`,
`main.rs`, `cli.rs`, and driver `frontend.rs`/`pipeline.rs`:

**Only `path`-kind dependencies are wired into compilation**, and even
those are handled by naive source concatenation, not real multi-crate
linking. `bundle_path_dependencies()` (`paths.rs`) walks path-typed
`DependencySpec::Inline(InlineDependency::Path)` entries recursively and
literally splices each dependency's bundled source into the compiled
buffer as an inline module:

```rust
out.push_str(&format!(
    "\n// auto-bundled dependency: {} ({})\nmod {} {{\n{}\n}}\n",
    dep_id, dep_root.display(), mod_name, dep_bundled
));
```

**No code path was found** in the CLI (`build.rs`, `check.rs`, `main.rs`,
`cli.rs`) or in `gossamer-driver` (`frontend.rs`, `pipeline.rs`) that reads
a `vendor/` directory or the global `~/.gossamer/cache` during
`build`/`run`/`check`/`test` to pull in registry, git, or tarball
dependency source. `gossamer-resolve/src/external.rs` (which the frontend
consults for external module lookups) is purely an in-memory table
populated by `set_external_modules` at process startup; its caller/data
source was not located in the crates read for this document.

**Practical implication:** as implemented today, `gos fetch` / `gos
vendor` / `project.lock` are a fully real fetch-and-lock pipeline, but
there is no confirmed mechanism by which their output actually reaches the
compiler for non-path dependencies. Either (a) this wiring exists
somewhere not covered by this reading and should be re-checked before
finalizing the adapter design, or (b) registry/git/tarball dependencies are
not yet consumable by `gos build` at all, and only `path`-style deps are
currently real for compilation purposes. **Treat this as the single
highest-priority thing to confirm against upstream (or upstream
maintainers) before committing to a `gossamer2nix` design** that assumes
`gos build --locked` can build a project with non-path dependencies
end-to-end inside a sandboxed derivation.

---

## 13. Deterministic package tarball format

`gos publish` packs via `pack_crate`/`pack_crate_streaming`
(`gossamer-pkg/src/publish.rs`), producing a byte-identical USTAR archive
for a given source tree:

- Recursively walks the project root, **excluding**: `target/`, `vendor/`,
  `.git/`, `.gos-cache/`, `.gos-bindings/`, `node_modules/`, dotfiles,
  `.DS_Store`, and `*.rs.bk` files.
- File entries emitted in **lexicographic order**.
- `mtime`/`uid`/`gid` are zeroed; mode normalized to `0o644`.
- End-of-archive marker is two 512-byte zero blocks (standard USTAR EOF).
- Symlinks are rejected (portability/security).
- `project.toml` is required at the root.
- Per-file and aggregate size limits enforced before reading contents.
- The computed sha256 is checked against the declared digest before
  upload.

Determinism here is what makes registry-source `sha256` pins in
`project.lock` reproducible across machines, directly relevant to
building a Nix fixed-output derivation around a fetched package.

---

## 14. `gos new` scaffolding (reference)

`gossamer-pkg/src/scaffold.rs` generates, for a fresh project:

`project.toml`:
```toml
[project]
id = "{id}"
version = "{version}"

[dependencies]
```

`src/main.gos`:
```
fn main() {
    println!("hello from {tail}")
}
```

(`println!` is one of a handful of macros always in scope without a `use`.)

---

## Sources

- `SPEC.md` §6 (Projects, Modules, `use`, dependency resolution, lockfile,
  registries) and §16 (toolchain command reference), in
  [danpozmanter/gossamer](https://github.com/danpozmanter/gossamer).
- `crates/gossamer-pkg/src/{manifest,lockfile,resolver,fetch,cache,version,
  id,transport,edit,scaffold,credentials,publish}.rs`
- `crates/gossamer-cli/src/{cli.rs,paths.rs,cmd/{pkg,build,check}.rs,main.rs}`
- `crates/gossamer-resolve/src/external.rs`
- `crates/gossamer-driver/src/{frontend,pipeline}.rs`

Read directly from upstream `main` on 2026-07-25 via GitHub's raw content
and contents APIs; no local clone of the gossamer repo exists in this
workspace. Sections 10 and 12 explicitly flag findings that could not be
fully confirmed from the excerpts read; re-verify those against current
upstream source (or ask upstream maintainers) before relying on them.
