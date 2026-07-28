# `zon2nix`: Bridging Zig's Package Manager into Nix

Read 2026-07-26 (`main`, release `v0.1.2`, all six `src/*.zig` files) as the closest prior art to `gossamer2nix`: a `*2nix` tool for a young, from-scratch-package-manager systems language, directly comparable to Gossamer's own `project.toml`/`project.lock` model (`../DEPS.md`).

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/zon2nix](https://github.com/nix-community/zon2nix) (`main`) |
| **Ecosystem** | Zig |
| **Maintenance status** | Active, small |
| **Ecosystem input** | `build.zig.zon` (ZON/Zig-struct syntax) — `.dependencies` entries already pinned to exact `{url, hash}`, recursively across every transitively-fetched dependency's own manifest |
| **Generated output** | `deps.nix` — a `linkFarm` of per-dependency fetchers, one entry per unique dependency |
| **Hash strategy** | Re-derived — Zig's own multihash (a `paths`-filtered directory hash) can't be reused; every dependency is independently re-fetched via `nix flake prefetch` for a real Nix SRI hash. The Zig hash survives only as a cache-directory key name |
| **Nix build mechanism** | `linkFarm` of per-dependency `fetchzip`/`fetchgit` FODs, symlinked into `$ZIG_GLOBAL_CACHE_DIR/p` so Zig's own toolchain finds them without touching the network |

## Input

`build.zig.zon` is parsed with Zig's own `std.zig.Ast` parser in ZON mode — `zon2nix` reuses the compiler's own AST parser rather than writing a bespoke grammar. Each `.dependencies` entry is either `{ .url, .hash }` (remote, fetched and hash-verified) or `{ .path }` (local; per upstream docs "the package's hash is irrelevant and therefore not computed" — and `zon2nix`'s own parser unconditionally requires `url`/`hash` to be present, so a manifest mixing path and remote deps looks like a hard parse error today, not confirmed against a real mixed manifest). `.lazy = true` (fetch-only-if-referenced) has no visible handling — `zon2nix` appears to fetch every declared dependency unconditionally, more eagerly than a real `zig build` would.

## Manifest and lockfile are the same file

The single most important structural fact: **`build.zig.zon` is not a manifest with version ranges resolved into a separate lockfile — every dependency entry is already pinned to an exact URL and exact content hash.** No version-range syntax, no registry index, no "pick the highest satisfying version" step. Per upstream docs: *"packages do not come from a `url`; they come from a `hash`. `url` is just one of many possible mirrors."* — a direct echo of Nix's own content-addressing philosophy.

`zon2nix` never invokes `zig`/`zig build` to resolve anything (confirmed: no subprocess spawn of a `zig` binary anywhere in source, only `nix flake prefetch` and an HTTP client for redirects). Instead it does the "resolution" itself as a breadth-first walk: prefetch every currently-known, not-yet-fetched dependency in parallel via `nix flake prefetch --json`, then open each fetched result's own `build.zig.zon` (if present) and parse it again, merging newly discovered dependencies into the same hash-keyed map, repeating until a full round adds nothing new. The "lockfile" is thus not centralized — it's distributed across every dependency's own manifest, and `zon2nix` reconstructs the full graph by literally walking the fetched tree.

## Why the hash still needs translating

Zig already does its own reproducible, content-addressed fetching: `.hash` is a multihash digest **"computed from the file contents of the directory of files that is obtained after fetching `url` and applying the inclusion rules given by `paths`"** — i.e. a hash of Zig's own canonicalized, `paths`-filtered directory listing. Nix's `fetchzip`/`fetchgit`, by contrast, hash the NAR serialization of whatever they unpack, with no `paths` allow-list. **These are two different hash functions over two different logical inputs that both happen to be "content-addressed."** That's exactly why `zon2nix` cannot simply reformat the Zig multihash into Nix's SRI form — the underlying digest would be wrong. It must independently re-fetch and re-hash every dependency:

```nix
{
  name = "122048992ca58a78318b6eba4f65c692564be5af3b30fbef50cd4abeda981b2e7fa5";  # Zig's multihash, verbatim, used only as a key
  path = fetchzip {
    url = "https://github.com/ziglibs/known-folders/archive/....tar.gz";
    hash = "sha256-U/h4bVarq8CFKbFyNXKl3vBRPubYooLxA1xUz3qMGPE=";  # independently computed by nix flake prefetch
  };
}
```

The Zig hash is used only as an identifier (the `linkFarm` entry name); the Nix SRI hash is what actually gives the fetch fixed-output-derivation integrity inside the sandbox. `codegen.zig` refuses to emit `deps.nix` if any dependency reaches codegen without a resolved Nix hash.

## Nix-side mechanism

The emitted `deps.nix` (`{ linkFarm, fetchzip, fetchgit }: linkFarm "zig-packages" [ { name; path; } ... ]`, sorted by Zig hash for deterministic output) is consumed via:

```nix
postPatch = ''
  ln -s ${callPackage ./deps.nix { }} $ZIG_GLOBAL_CACHE_DIR/p
'';
```

Zig's build system, resolving a dependency by hash, looks for a pre-populated `p/<hash>/` entry under its global cache directory before attempting any network fetch. `zon2nix`'s `linkFarm` produces exactly that shape, so `zig build` finds what it expects without touching the network. Not independently confirmed: whether Zig re-validates a pre-populated cache entry's content against the expected hash on use, or simply trusts presence-by-name — a general web search suggests the latter (load-and-parse-if-present), which would make the actual trust boundary "Nix fetched *something* reproducibly and put it at the name Zig expects," not a cryptographic proof the fetched tree matches what the hash was computed over.

## Why the tool exists

Given Zig already does native content-addressed fetching, the gap is narrower than in most ecosystems, but real: (1) `zig build`/`zig fetch` hit the network directly; Nix's sandbox forbids that outside an FOD, so the fetch has to move into a `nix flake prefetch`-driven, ahead-of-time `deps.nix` generation step. (2) The hash-scheme incompatibility (above) means this is a genuine re-fetch-and-rehash, not just a re-encoding. (3) Even with no constraint-solving to replicate, there's still a real recursive discover-and-fetch walk, since the full dependency set isn't enumerable from the root manifest alone.

## Limitations

- **No `.path`-dependency support** observed — likely a hard parse error on manifests mixing local and remote deps, unconfirmed against a real case.
- **No `.lazy` handling** — fetches more eagerly than a real `zig build` would.
- **FOD hash churn is an active nixpkgs maintenance pain**: a nixpkgs maintainer describes `zon2nix`-style per-dependency FOD breakage as "a nightmare," because "Zig is a pre-1.0.0 language and there's seemingly no guarantee of FODs staying stable." An alternative (`zig.fetchDeps`, wrapping the whole `zig build --fetch` in one FOD) was proposed as more robust, but most Zig packages in nixpkgs still use `zon2nix` today regardless — an unresolved tension, not settled practice.
- **Requires network access to generate** `deps.nix` — not sandboxed or reproducible-by-construction itself; a developer/CI-time tool only.
- **Heuristic ref/rev disambiguation**: distinguishing a commit hash from a branch/tag name in a URL fragment is done by an all-hex-characters check the source's own test comments admit "may be a bit brittle."

## Comparison with Gossamer's model

Zig's manifest/hash/resolution model is structurally simpler than Gossamer's at every layer that matters here, and that simplicity is the direct cause of `zon2nix`'s small scope. Zig conflates manifest and lockfile into one file — no version ranges, no registry protocol, no signature/publisher-trust model, and critically **no resolution algorithm to replicate**; `zon2nix` only walks an already-fully-determined graph and re-fetches each node once. Gossamer spreads the same job across a separate manifest (`project.toml`, caret ranges, `[registries]`), a real greedy non-backtracking resolver (`../DEPS.md` §6), a distinct lockfile (`project.lock`), a registry HTTP protocol with Ed25519 publisher-signature verification (§10), and a local cache with its own admission policy (§9) — and per `../DEPS.md` §12, `gos build` doesn't yet appear to consume any of that for non-`path` dependencies at all, a question with no Zig-side analog since Zig's own dependency-consumption path is real and exercised today.

**Implication**: if Gossamer's registry/git/tarball path turns out to be real and load-bearing, `gossamer2nix`'s job is substantially harder than `zon2nix`'s — it must reimplement Gossamer's specific greedy resolver (not just walk an already-pinned graph), handle a real registry protocol and signature-trust model as a first-class fetch-time concern, and reconcile a lockfile where the content digest is recorded *alongside* the source identifier rather than *as* it (closer in spirit to `Cargo.lock` than to `build.zig.zon`). Conversely, if `gos build` genuinely only wires in `path`-kind dependencies today (per §12), `gossamer2nix`'s real-world initial scope could ironically end up narrower than `zon2nix`'s — path deps need no fetching, hashing, or lockfile machinery at all, and the resolver/registry/signature work would be dead code from a Nix builder's perspective until upstream wires it in.
