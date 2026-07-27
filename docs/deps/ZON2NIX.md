# `zon2nix`: Bridging Zig's Package Manager into Nix

This document was compiled by reading the full source of [nix-community/zon2nix](https://github.com/nix-community/zon2nix) (`main` branch, all six `src/*.zig` files, `README.md`, `flake.nix`, `nix/package.nix`, current release `v0.1.2`), the upstream Zig manifest spec [`doc/build.zig.zon.md`](https://github.com/ziglang/zig/blob/master/doc/build.zig.zon.md), and the NixOS Discourse thread ["How should we package Zig software?"](https://discourse.nixos.org/t/how-should-we-package-zig-software/77409), all fetched **2026-07-26**.

It exists to ground this repo's `gossamer2nix` design (see [`../GOALS.md`](../GOALS.md)) in the closest existing prior art: a `*2nix` tool for a young, from-scratch-package-manager systems language, directly comparable to [`../DEPS.md`](../DEPS.md)'s documentation of Gossamer's own `project.toml`/`project.lock` model. Read `DEPS.md` first — this document assumes familiarity with it and compares against it directly in the final section.

**This reflects one point-in-time reading of upstream `main`/`v0.1.2` and may drift.** Zig's own package-manifest format and hash scheme are explicitly pre-1.0 and have changed before (see §6); re-verify before depending on exact details.

---

## 1. What `zon2nix` is

A small Zig program (not a shell script or Python tool) that reads a Zig project's `build.zig.zon` manifest, resolves and fetches its full dependency tree via `nix flake prefetch`, and prints a self-contained Nix expression (conventionally saved as `deps.nix`) that a downstream package build can consume to populate Zig's own dependency cache **without network access**, inside a Nix sandbox.

Maintained by `@figsoda` under the `nix-community` GitHub org, MPL-2.0 licensed, ~122 stars. Usage per `README.md`:

```bash
zon2nix > deps.nix
zon2nix zls > deps.nix
zon2nix zls/build.zig.zon > deps.nix
```

Source layout (`src/`): `main.zig` (entry point/orchestration), `parse.zig` (ZON manifest parsing), `fetch.zig` (drives `nix flake prefetch` and transitive discovery), `Dependency.zig` (URL/ref parsing + `nix flake prefetch` ref formatting), `codegen.zig` (emits the final Nix expression), `http.zig` (HTTP redirect resolution).

---

## 2. Input: `build.zig.zon`

Per upstream `doc/build.zig.zon.md`, `build.zig.zon` is a **ZON** (Zig Object Notation) file — Zig struct-literal syntax, not TOML/JSON/YAML — parsed by `zon2nix` using Zig's own `std.zig.Ast` parser in ZON mode (`parse.zig`: `Ast.parse(alloc, content, .zon)`), i.e. `zon2nix` reuses the Zig compiler's own AST parser rather than writing a bespoke ZON grammar.

Top-level manifest fields (per upstream doc, not all consumed by `zon2nix`):

| Field | Type | Notes |
|---|---|---|
| `.name` | enum literal | package identity component (paired with `.fingerprint`) |
| `.fingerprint` | 64-bit int | auto-generated, "id + checksum"; detects fork/rename vs. genuinely new package |
| `.version` | semver string | |
| `.minimum_zig_version` | semver string | advisory only, not enforced by the compiler |
| `.dependencies` | struct | the only part `zon2nix` reads (see below) |
| `.paths` | list, required | files/dirs included in the package; **only these are hashed** (see §4) |

Each entry in `.dependencies` is one of:

- `{ .url = "...", .hash = "..." }` — remote dependency, fetched and hash-verified.
- `{ .path = "..." }` — local relative-path dependency; "the package's hash is irrelevant and therefore not computed" (upstream doc). `zon2nix`'s `parse.zig` only reads `url`/`hash` fields per entry and returns `error.parseError` if both aren't present — **path-kind dependencies are not handled by `zon2nix` at all** (unconfirmed whether they're silently skipped or the whole parse fails when a `.path` dep exists in a real manifest that mixes both kinds; the source shown requires `url != null and hash != null` unconditionally, so this looks like a hard error path today — flag for re-verification against a real mixed manifest).
- `.lazy = true` optionally marks a dependency as fetched only if actually referenced by `build.zig`; `zon2nix`'s parser has no visible handling of `.lazy` — it appears to fetch every declared dependency unconditionally, which would be *more eager* than a real `zig build`.

The `.url` field itself has three sub-shapes `Dependency.zig`'s `DependencyUrlParseResult.parseUrl` distinguishes:

- Plain tarball URL: `https://...`
- Git with explicit ref/tag fragment: `git+https://host/repo#branch-or-tag-name`
- Git with commit fragment: `git+https://host/repo#<hex-sha>` (distinguished from the ref case by whether the fragment is all-hex, per a source comment acknowledging this heuristic "may be brittle")

Example manifest (from `README.md`, originally `zls`'s):

```zig
.{
    .name = "zls",
    .version = "0.11.0",
    .dependencies = .{
        .known_folders = .{
            .url = "https://github.com/ziglibs/known-folders/archive/fa75e1bc672952efa0cf06160bbd942b47f6d59b.tar.gz",
            .hash = "122048992ca58a78318b6eba4f65c692564be5af3b30fbef50cd4abeda981b2e7fa5",
        },
        ...
    },
}
```

---

## 3. Manifest and lockfile are the same file — no separate resolver step

This is the single most important structural difference from most package ecosystems, Gossamer included: **`build.zig.zon` is not a manifest with version ranges that get resolved into a separate lockfile. Each `.dependencies` entry is already pinned to an exact URL and an exact content hash.** There is no version-range syntax to resolve, no registry index to query, no "pick the highest satisfying version" step. Per upstream doc: **"This field \[`.hash`\] is the source of truth; packages do not come from a `url`; they come from a `hash`. `url` is just one of many possible mirrors for how to obtain a package matching this `hash`."** That sentence is a direct, explicit echo of Nix's own content-addressing philosophy.

Consequently `zon2nix` **never invokes `zig` or `zig build`** to resolve anything (confirmed by reading all of `src/`: no `std.process.Child` spawn of a `zig` binary anywhere, only `nix flake prefetch` and an HTTP client for redirect-following). Instead, `fetch.zig`'s `fetch()` does the entire "resolution" itself as a breadth-first work-queue:

1. For every dependency currently known and not yet `done`, spawn `nix flake prefetch --json --extra-experimental-features 'flakes nix-command' <ref>` in parallel (one child process per dependency, all launched before any is awaited — a real parallel fan-out, not a sequential loop).
2. Parse each result's JSON (`{hash, storePath}`).
3. **Recurse structurally, not via a registry**: after each successful prefetch, `fetch.zig` opens `<storePath>/build.zig.zon` if present and calls `parse()` on it again, merging newly discovered dependencies into the same `StringHashMap(Dependency)`, keyed by the Zig multihash string. The outer `while (!done)` loop repeats until a full round adds no new dependency count.
4. Duplicate hashes across different consumers collapse into a single map entry automatically (it's a hash-keyed map); in debug builds, `parse.zig` additionally re-derives the dependency from its URL and asserts the URL agrees with the already-recorded one for that hash, as an internal consistency check (not a security feature — just a developer sanity assertion).

So the "lockfile" is not a single centralized file the way `project.lock` is for Gossamer — **it is distributed across every dependency's own `build.zig.zon`**, and `zon2nix` reconstructs the full flattened graph by literally walking the fetched tree of manifests, each one already self-pinned. This works precisely because Zig's manifest format conflates what Gossamer splits into two files and two mechanisms (`project.toml`'s caret ranges + a greedy non-backtracking resolver in `gossamer-pkg/src/resolver.rs`, versus `project.lock`'s recorded pins).

---

## 4. Zig's own hash scheme vs. Nix's — why translation is still real work

Zig already does its own reproducible, content-addressed dependency fetching (`src/Package/Fetch.zig` upstream, not re-read in full for this document — flagged as unconfirmed in exact algorithmic detail, but its output format is well-documented): the `.hash` field is a [multihash](https://multiformats.io/multihash/)-formatted digest (the `1220...` values in the example above use the `0x12 0x20` sha2-256 multihash prefix), described by upstream as **"computed from the file contents of the directory of files that is obtained after fetching `url` and applying the inclusion rules given by `paths`."**

This matters because it means the Zig hash is *not* a hash of the raw tarball/git-tree bytes — it's a hash of Zig's own canonicalized, `paths`-filtered directory listing. Nix's `fetchzip`/`fetchgit`, by contrast, compute their fixed-output hash over the NAR serialization of whatever they unpack, with no knowledge of a `paths` allow-list. **These are two different hash functions over two different logical inputs that happen to both be "content-addressed."** That is precisely why `zon2nix` cannot simply reformat the Zig multihash into Nix's SRI (`sha256-...`) base64 form — the underlying digest itself would be wrong. It must **independently re-fetch and re-hash** every dependency via `nix flake prefetch`, exactly as `fetch.zig` does.

The example in `README.md` shows this concretely — the emitted `deps.nix` for the `known_folders` dependency uses:

```nix
{
  name = "122048992ca58a78318b6eba4f65c692564be5af3b30fbef50cd4abeda981b2e7fa5";  # Zig's own multihash, verbatim
  path = fetchzip {
    url = "https://github.com/ziglibs/known-folders/archive/fa75e1bc672952efa0cf06160bbd942b47f6d59b.tar.gz";
    hash = "sha256-U/h4bVarq8CFKbFyNXKl3vBRPubYooLxA1xUz3qMGPE=";  # independently computed by `nix flake prefetch`, unrelated bytes
  };
}
```

The Zig hash and the Nix hash are **different digests of different things**, both present, playing different roles:

- The Zig multihash is used **only as an identifier/key** — the `linkFarm` entry's `name`, which becomes a directory name.
- The Nix SRI hash is what actually gives the `fetchzip`/`fetchgit` call fixed-output-derivation integrity inside the Nix sandbox.

This is the crux of §5 below: the directory name has to equal the Zig-computed hash exactly, because that's the name Zig's own toolchain will look for later; the Nix hash has to be an independently and correctly computed Nix content hash, because that's what Nix's sandbox enforces. `codegen.zig`'s `write()` errors out (`error.MissingNixHash`) if any dependency reached codegen without a resolved `nix_hash` — i.e. it refuses to emit a `deps.nix` with a placeholder or stale hash.

Two dependency shapes are emitted, matching the two `Dependency.Parameters` union variants populated in `Dependency.zig`:

- No ref/rev (plain URL) → `fetchzip { url; hash; }`
- Git with a `rev` (commit) or `ref` (branch/tag) fragment → `fetchgit { url; rev|ref; hash; }`

`Dependency.flakePrefetchRef()` builds the actual string passed to `nix flake prefetch`, translating Zig's own URL-with-fragment convention (`git+https://host/repo?query#fragment`) into Nix's flake-ref convention (`git+https://host/repo?rev=...` or `?ref=refs/tags/...`, with the `refs/tags/` prefix added specifically to work around [NixOS/nix#5291](https://github.com/NixOS/nix/issues/5291) on older `nix` versions), or a bare `tarball+https://...` for plain URLs. It also resolves HTTP redirects itself (`http.zig`) before handing a URL to `nix`, presumably to get a stable canonical URL into the generated Nix expression rather than a redirecting one.

---

## 5. Output: `deps.nix` and cache-directory integration

The emitted file (verbatim structure, per `codegen.zig` and the `README.md` example):

```nix
# generated by zon2nix (https://github.com/nix-community/zon2nix)

{ linkFarm, fetchzip, fetchgit }:

linkFarm "zig-packages" [
  {
    name = "<zig-multihash-of-this-dependency>";
    path = fetchzip { url = "..."; hash = "sha256-..."; };  # or fetchgit { url; rev|ref; hash; }
  }
  ...
]
```

Entries are sorted lexicographically by the Zig hash string (`mem.sortUnstable` in `codegen.zig`) for deterministic, diff-friendly output.

To consume it, `README.md` prescribes:

```nix
postPatch = ''
  ln -s ${callPackage ./deps.nix { }} $ZIG_GLOBAL_CACHE_DIR/p
'';
```

This is the actual bridge mechanism: Zig's build system, when resolving a dependency by hash, looks for a pre-populated `p/<hash>/` entry under its global cache directory (`$ZIG_GLOBAL_CACHE_DIR`, per upstream Zig's persistent-cache design) before attempting any network fetch. `zon2nix`'s `linkFarm` produces exactly that `p/`-shaped directory (one symlink per dependency, named by its Zig hash, pointing at a Nix store path fetched deterministically), and `postPatch` symlinks the whole farm into place before `zig build` ever runs. Because the directory names are the exact Zig multihashes, Zig's toolchain finds what it's looking for without touching the network — this is what makes the resulting package buildable inside Nix's network-disabled sandbox.

**Not independently confirmed in this reading**: exactly how strictly Zig re-validates a pre-populated `p/<hash>` cache entry against its expected hash at build time (i.e., does it re-hash the directory contents and compare, or does it trust presence-by-name once `build.zig.zon`-in-that-directory parses successfully?). A general web search on Zig's `src/Package/Fetch.zig` behavior surfaced only that "the global zig package cache checks if a hash already exists, and if so, loads, parses, and validates the `build.zig.zon` file therein" — consistent with trust-by-directory-name rather than a full re-hash, but this was not verified by reading `Fetch.zig` directly. If Zig does *not* re-hash on cache hit, then `zon2nix`'s guarantee is really "Nix fetched *something* reproducibly and put it at the name Zig expects," not a cryptographic proof that the fetched tree matches what the original hash was computed over — worth flagging as a trust-boundary nuance, not a defect necessarily, but a meaningfully different guarantee than Gossamer's own fetch-time signature+digest verification (`DEPS.md` §8).

---

## 6. Why this tool exists at all, and its actual limitations

Given Zig already does native content-addressed fetching, the gap `zon2nix` fills is narrower than in most other ecosystems, but it is real and has three distinct parts, all confirmed above:

1. **Sandbox/network mismatch, not a hash-format mismatch alone.** Zig's own `zig build`/`zig fetch` happily hits the network directly; Nix's build sandbox forbids that (except inside a fixed-output derivation). `zon2nix` exists to move the network fetch *out* of the build (into `nix flake prefetch`, itself a set of FODs) and *into* a `deps.nix` generation step run ahead of time by a developer or CI, then committed — exactly the same "run the `*2nix` tool once, commit the output" workflow as `gomod2nix`/Cargo-vendoring tools.
2. **Genuine hash-scheme incompatibility, not just encoding.** As detailed in §4, Zig's `.hash` is computed over a `paths`-filtered, Zig-canonicalized file listing; Nix's FOD hash is computed over a NAR of whatever `fetchzip`/`fetchgit` produces. These are different digests of different byte streams, so `zon2nix` must actually re-fetch-and-rehash via `nix flake prefetch`, not just re-encode a hex string into SRI base64. The Zig hash survives only as an opaque cache-directory key, not as cryptographic proof accepted by Nix.
3. **Self-describing manifest, but still a real (if simple) graph walk.** Because every dependency is already pinned (§3), there's no *constraint-solving* work to replicate — but there is still a nontrivial recursive-fetch-and-discover walk, because the full dependency set isn't enumerable from the root manifest alone (transitive dependencies' manifests are only known after fetching their parents).

**Limitations/gotchas, confirmed or reported:**

- **No `.path`-dependency support** observed in `parse.zig` (§2) — a manifest depending on a local path dependency alongside hashed ones is, on this reading, unhandled or a hard parse error; not confirmed against a real mixed-kind manifest.
- **No `.lazy` handling** — `zon2nix` appears to eagerly fetch every declared dependency regardless of `.lazy = true`, which could mean `deps.nix` fetches (and thus rebuilds/redownloads on hash changes) more than a real `zig build` invocation would need.
- **FOD hash churn is a known, actively-discussed maintenance pain in nixpkgs.** Per the Discourse thread "How should we package Zig software?" (2026), nixpkgs maintainer `water-sucks` describes "FOD hash breakage" from `zon2nix`-style per-dependency fixed-output derivations as "a nightmare when it comes to maintenance," explicitly because "Zig is a pre-1.0.0 language and there's seemingly no guarantee of FODs staying stable" — i.e., upstream Zig dependency hosts/hash schemes have changed in the past, invalidating previously-generated `deps.nix` files. An alternative approach discussed there, `zig.fetchDeps` (letting `zig` itself do the fetching inside a single Nix FOD wrapping the whole `zig build --fetch` step, rather than `zon2nix` doing per-dependency FODs) was proposed as more robust, but per that same thread, **"the majority of Zig software \[...\] packaged in nixpkgs is by using zon2nix"** regardless, for practical/momentum reasons — flagged as an unresolved tension in the Zig-on-Nix ecosystem, not a settled best practice.
- **Requires network access to *generate* `deps.nix`** (parallel `nix flake prefetch` processes plus raw HTTP redirect-resolution requests in `http.zig`) — this step itself is not sandboxed or reproducible-by-construction the way the *consumption* of the resulting `deps.nix` is; it's a developer/CI-time tool, not something that runs inside a `nix build` sandbox itself.
- **Heuristic ref/rev disambiguation**: `Dependency.zig`'s own test comments admit the "is this fragment a commit hash or a branch/tag name" check (all-hex-characters test) "may be a bit brittle."
- Zig's manifest/hash format has changed before pre-1.0 (an evolving multihash-based scheme, with newer/alternate hash-format proposals tracked upstream, e.g. [ziglang/zig#20178](https://github.com/ziglang/zig/issues/20178) — not independently verified against the current `v0.1.2` `zon2nix` release for which exact hash-format version it assumes).

---

## Input / Output summary

**Input** (ecosystem-native, consumed by `zon2nix`):
- One `build.zig.zon` file (ZON/Zig-struct syntax) per package, whose `.dependencies` table is *already* a set of exact pins: `{url, hash}` pairs (remote) or `{path}` (local, unsupported by `zon2nix` per §2/§6) — no version ranges, no registry, no separate lockfile. Transitively, every fetched dependency's own `build.zig.zon` is also consumed recursively.
- Live network access (HTTP + `nix flake prefetch`, itself hitting git/tarball hosts) at `zon2nix` run time, to discover and hash the full transitive graph.

**Output** (Nix-consumable, produced by `zon2nix`):
- A single `deps.nix` file: `{ linkFarm, fetchzip, fetchgit }: linkFarm "zig-packages" [ ... ]`, one entry per unique dependency, `name` = the original Zig multihash string, `path` = a `fetchzip`/`fetchgit` fixed-output derivation with an **independently Nix-computed** SRI hash (not derived from the Zig hash).
- No resolver output, no separate lockfile artifact — `deps.nix` *is* the lockfile-equivalent, generated fresh each run from the (already-pinned) manifest graph, meant to be committed to the consuming package's repo and wired in via `postPatch`'s symlink into `$ZIG_GLOBAL_CACHE_DIR/p`.

---

## 7. Comparison with Gossamer's model

Zig's manifest/hash/resolution model is **structurally simpler than Gossamer's at every layer that matters for a `*2nix` tool**, and that simplicity is the direct cause of `zon2nix`'s modest size (~29 KB of Zig source across six files) and narrow scope. Zig conflates manifest and lockfile into a single file per package (§3): every dependency entry in `build.zig.zon` is already an exact `{url, hash}` pin, so there is no version-range syntax, no registry protocol, no signature/publisher-trust model, and — critically — **no resolution algorithm to replicate**. `zon2nix` never has to reimplement anything like Gossamer's greedy, non-backtracking, descending-version resolver (`DEPS.md` §6); it only has to walk an already-fully-determined graph and re-fetch each node once to obtain a Nix-native hash, because Zig's own content hash (§4) — despite also being cryptographically strong and content-addressed — is computed by a different, `paths`-filtered algorithm than Nix's NAR-based FOD hash and so can't be reused verbatim. Gossamer, by contrast, spreads the same job across a full separate manifest (`project.toml`, caret ranges, `[registries]`), a real resolver (`gossamer-pkg/src/resolver.rs`), a distinct lockfile (`project.lock`, `DEPS.md` §7), a registry HTTP protocol with Ed25519 publisher-signature verification (`DEPS.md` §10), and a local content-addressed cache with its own admission/eviction policy (`DEPS.md` §9) — and, per `DEPS.md` §12, Gossamer's own `gos build` doesn't yet appear to consume any of that machinery for non-`path` dependencies at all, an open question with no Zig-side analog since Zig's dependency-consumption path (the `$ZIG_GLOBAL_CACHE_DIR/p` lookup `zon2nix` targets) is a real, exercised part of `zig build` today.

**Implication for `gossamer2nix`:** if Gossamer's registry/git/tarball dependency path does turn out to be real and load-bearing for `gos build` (the open question flagged in `DEPS.md` §12), `gossamer2nix`'s job will be substantially harder than `zon2nix`'s along at least three axes `zon2nix` never has to deal with: (a) it must faithfully **reimplement Gossamer's specific greedy resolver** (or otherwise guarantee bit-identical resolution decisions) rather than just walking an already-pinned graph, since `project.toml` alone is not self-describing the way `build.zig.zon` is; (b) it must handle a real **registry protocol and signature-trust model** (`[trusted-publishers]`, Ed25519 tarball signatures) as a first-class fetch-time concern, not just a re-hash; and (c) unlike Zig's `.hash`-is-truth model where the URL is explicitly "just a mirror," Gossamer's `project.lock` pins carry source-kind-specific fields (`version`/`url+ref`/`path`) with the content digest (`sha256`) recorded *alongside* rather than *as* the primary identifier — closer in spirit to Cargo's `Cargo.lock` than to Zig's manifest. Conversely, if `DEPS.md` §12's finding holds and `gos build` genuinely only wires in `path`-kind dependencies today, then `gossamer2nix`'s *initial* real-world scope could ironically end up narrower than `zon2nix`'s, since path dependencies need no fetching, hashing, or lockfile machinery at all — the hard resolver/registry/signature work described in `DEPS.md` §§6–10 would be dead code from a Nix-builder's perspective until upstream actually wires it into compilation.
