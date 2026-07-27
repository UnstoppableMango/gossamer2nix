# `gossamer2nix` Design

This document proposes a design for `gossamer2nix`'s deps-lock/Nix adapter, synthesizing lessons from eight prior-art `*2nix` tools (`GOMOD2NIX.md`, `CRATE2NIX.md`, `NAERSK.md`, `BUN2NIX.md`, `PNPM2NIX.md`, `POETRY2NIX.md`, `PIP2NIX.md`, `ZON2NIX.md` in this directory) against Gossamer's actual dependency model, as documented from upstream source in [`../DEPS.md`](../DEPS.md).

**This is a proposal, not an implementation plan with committed dates.** It is gated on one unresolved upstream question (§0) that determines how much of it is buildable at all today.

---

## 0. The gating question

[`DEPS.md` §12](../DEPS.md) found no code path in `gossamer-cli`/`gossamer-driver` that reads `vendor/` or the package cache during `gos build`/`run`/`check`/`test` — only `path`-kind dependencies are actually wired into compilation today, via naive source-concatenation (`bundle_path_dependencies`). Registry/git/tarball dependencies have a fully real fetch/lock/cache pipeline (`gos fetch`, `project.lock`, content-addressed cache) but no *confirmed* consumer at build time.

Every design decision below about registry/git/tarball handling is written as if that wiring exists or will exist, because that's the only way that a `gossamer2nix` supporting non-`path` dependencies would be worth building. But **the first concrete deliverable should be a smoke test that settles this empirically** (extend `nix/checks.nix`'s existing `hello-app` pattern with a second check that adds a registry dependency and see whether `gos build --locked` actually pulls it in), before investing in the fetch/hash machinery in §2–§4. If it turns out non-`path` deps aren't consumable yet, `gossamer2nix`'s real-world scope today is just what `nix/builder.nix` already does — and this document's §2–§4 becomes work to pick up once upstream wires it in, not work to do now.

---

## 1. What every prior-art tool agrees on (the parts not worth relitigating)

Reading eight of these tools converges on a small set of patterns strongly enough that `gossamer2nix` should just adopt them rather than search for something cleverer:

1. **Trust the ecosystem's own lockfile; never re-resolve.** `crate2nix`, `naersk`, `pnpm2nix`, `bun2nix`, and `poetry2nix` all take an already-resolved lockfile as ground truth and do zero dependency resolution of their own. The one tool that doesn't have a lockfile to trust — `pip2nix`, forced to re-run pip's own internal (unversioned, repeatedly-broken) resolver live against the network — is also the clearest cautionary tale in the set (§2 of `PIP2NIX.md`: a decade of `try/except ImportError` chains chasing pip's internals). Gossamer already has a real, algorithmically-specified, non-backtracking resolver (`DEPS.md` §6) and a lockfile it produces (`project.lock`, `DEPS.md` §7). `gossamer2nix` should consume `project.lock` directly and never reimplement `gossamer-pkg::resolver`. This is a correctness win (no risk of disagreeing with `gos`'s own resolution) and an engineering-effort win (no resolver to write or keep in sync).

2. **Separate "fetch" (network-permitted) from "build" (network-free).** Every tool surveyed does some version of: turn each dependency into its own Nix fixed-output derivation (network allowed, hash-checked), then run the ecosystem's real build/install tool fully offline against the fetched, pre-verified results. `gomod2nix` and `crate2nix` do this per-module/per-crate; `naersk` and `bun2nix`/`pnpm2nix` do it via Cargo/Bun/pnpm's own source-replacement or cache mechanisms. This is simply how Nix sandboxing and network-dependent package managers get reconciled; there's no third approach worth inventing.

3. **Path/local dependencies never get fetched or hashed.** Universal across every tool that has a path-dependency concept (`crate2nix`'s `LocalDirectory`, `naersk`'s local `path` overrides via `additionalCargoLock`, `poetry2nix`'s `directory`/`file` sources, `zon2nix`'s unsupported-but-acknowledged `.path`). Gossamer's own `{ path = ... }` kind is unhashed in `project.lock` for the same reason (`DEPS.md` §7). `gossamer2nix` should symlink these directly, matching what `gos build`'s own path-bundling already does.

4. **Don't assume a hash format transfers for free — verify it, per source kind.** This is the one area where the eight tools *split*, and the split is exactly the thing worth deciding deliberately rather than copying a default (§2 below).

---

## 2. The central design question: does Gossamer's `sha256` reuse as a Nix hash?

This is the single most consequential unknown, and it's worth being explicit that the prior art splits roughly in half:

**Reuses verbatim, no conversion needed** (the ecosystem's own hash happens to already be a hash Nix's fetchers accept):
- `crate2nix` / `naersk`: `Cargo.lock`'s `checksum` is plain SHA-256 hex of the crates.io tarball bytes — exactly the legacy hex form `fetchurl`'s `sha256` argument accepts.
- `pnpm2nix` / `bun2nix` (npm packages only): the lockfile's `integrity`/`resolution.integrity` field is already SRI (`sha512-<base64>`), which Nix's fetchers accept natively.
- `poetry2nix`: `poetry.lock`'s `hash = "sha256:<hex>"` is already in Nix's `algo:hex` hash grammar — explicitly called out in `POETRY2NIX.md` §5.1 as "a syntactic no-op... coincidence," not something to assume generalizes.

**Cannot be reused — must independently refetch and rehash**:
- `gomod2nix`: `go.sum`'s `h1:` dirhash is a SHA-256 over a *sorted list of per-file digests* (Go's own scheme); Nix's FOD hash is a NAR (recursive binary directory serialization). Different algorithm over different logical input — `go.sum` is read into memory and then never used.
- `zon2nix`: Zig's own multihash is computed over a `paths`-filtered, Zig-canonicalized directory listing; Nix's FOD hash is a NAR of whatever `fetchzip`/`fetchgit` produces. Also cryptographically strong, also content-addressed, still incompatible — `zon2nix` re-fetches via `nix flake prefetch` regardless and keeps the Zig hash only as a cache-directory key, not as fetch integrity.
- `pip2nix` / `bun2nix` (git & tarball deps): no hash exists in the source format at all for these kinds, so the tool must mint one itself via a live prefetch (`nix-prefetch-url`/`nix flake prefetch`) at generation time.

**Where does Gossamer's `sha256` land?** Per `DEPS.md` §7, it's described as "the content-addressed digest of the *fetched and verified source tree*," recorded by `gos fetch`/`gos update`. Its exact serialization algorithm was not confirmed while reading `gossamer-pkg` in the original research pass — and there is no reason to assume it happens to be a NAR hash (nothing in the ecosystem's design gives it a reason to be; SHA-256-over-a-tarball, SHA-256-over-a-sorted-manifest, and NAR-of-a-directory are all equally plausible choices for an implementer who has never heard of Nix).

**Design decision: assume it does not transfer, and follow the `gomod2nix` pattern.** This is the conservative choice, and it degrades gracefully — if it later turns out Gossamer's hash *does* happen to be NAR-compatible, the fallback path (§3, "independent rehash") still works, just with an unnecessary extra fetch; if it doesn't transfer and `gossamer2nix` assumed it did (the `poetry2nix` mistake, except poetry2nix got lucky), every generated lockfile is silently wrong. **Before committing this to code, `gossamer2nix generate` should compute both** (Gossamer's own recorded `sha256` from `project.lock`, and an independently-computed NAR+SHA-256 over the same fetched tree) **and compare them once**, empirically, the same way this whole research effort favored reading real source over assumption. If they match, the conversion is free (reformat hex→SRI) and a whole fetch-and-rehash step disappears from the design. Don't guess; check.

---

## 3. Proposed architecture

### 3.1 `gossamer2nix generate`

A CLI (language TBD — Rust is a natural fit given the existing `[rust-bindings]` FFI overlap and prior art's own preferences, but this is not load-bearing) that:

1. Reads `project.lock` (not `project.toml` — per §1.1, the lock is the resolved ground truth). Never invokes or reimplements `gossamer-pkg::resolver`.
2. For each `[[project]]` entry, dispatches on `source`:
   - **`path`**: emit a passthrough entry with no hash — just the relative path, mirroring `gos`'s own `bundle_path_dependencies` semantics (`DEPS.md` §12) and every prior-art tool's identical treatment of local deps (§1.3).
   - **`registry`**: needs a real fetch to hash. Rather than reimplementing Gossamer's registry protocol and Ed25519 signature verification in the generator (an unconfirmed, partially-undocumented protocol per `DEPS.md` §10) — shell out to `gos fetch` (or a narrower equivalent, if one gets added upstream, e.g. `gos fetch --package <id>@<version>`) and let it do the verification it already does. This mirrors `gomod2nix` delegating entirely to `go mod download --json` rather than reimplementing the Go module proxy protocol (`GOMOD2NIX.md` §1) — trust the ecosystem's own tool for authenticity, and only add a Nix-native hash on top of what it already verified.
   - **`git`**: Gossamer's fetch step already requires locked git refs to be full 40/64-hex commit ids (`DEPS.md` §8) — i.e. every git dependency is already pinned to an immutable commit by the time it reaches `project.lock`, which is exactly what a Nix `fetchgit`/`builtins.fetchGit` wants as a `rev`. This is a genuinely easy case, closer to `naersk`'s git handling (§3 of `NAERSK.md`: `builtins.fetchGit` keyed directly off the pinned revision, no separate Nix-level hash needed at all) than to `gomod2nix`'s.
   - **`url` (tarball)**: Gossamer's manifest already mandates a `sha256` for these at the `project.toml` level (`DEPS.md` §3, "there is no unchecksummed URL dependency") — check whether that manifest-level hash is *the same* `sha256` recorded in `project.lock`, or something else; if the same, this is the one dependency kind where Gossamer's own author already had to solve "does my hash match what I fetched," and might be the cheapest place to test §2's hash-compatibility question first.
3. Writes a generated lockfile (working name `gossamer2nix.toml`, deliberately mirroring `gomod2nix.toml`'s shape) — one entry per non-path dependency, each carrying enough to redo the fetch (source kind, url/ref/version) plus a Nix-native hash (`sha256-<base64>` SRI, per §2's resolution).
4. **Reuses the previous generated lockfile's hash for unchanged `(id, pin)` pairs** rather than re-fetching every dependency on every run — `gomod2nix`'s explicit, deliberate trust/performance tradeoff (`GOMOD2NIX.md` §2, §5). Worth naming as a deliberate choice here too, not defaulting into it silently.

### 3.2 Nix-side: per-dependency fixed-output derivations

Extend `nix/builder.nix` (or a new `nix/deps.nix`) with a `fetchGossamerDependency`-shaped function, directly modeled on `gomod2nix`'s `fetchGoModule` (`GOMOD2NIX.md` §4):

```nix
fetchGossamerDependency = { hash, id, source, ... }@dep:
  stdenvNoCC.mkDerivation {
    name = "${baseNameOf id}-fetch";
    builder = ./fetch-dependency.sh;
    outputHashMode = "recursive";
    outputHash = hash;              # from gossamer2nix.toml
    # network allowed here (FOD exception), forbidden everywhere else
  };
```

`fetch-dependency.sh` re-runs, per source kind, whatever `generate` (§3.1) did to produce the original hash — re-invoke `gos fetch` for registry deps, `fetchgit`-equivalent for git deps — and copies the result to `$out`. Nix then hashes `$out` and compares against the declared `outputHash`; a mismatch fails the build. This is the actual reproducibility enforcement, exactly as in every prior-art tool: the generated lockfile's hash is a *promise*, checked by Nix itself at build time, not blindly trusted.

**Filter parity is load-bearing.** `gomod2nix`'s one universally-applicable gotcha (`GOMOD2NIX.md` §5): whatever filtering is applied to a fetched tree before hashing at `generate` time must be *bit-for-bit identical* to the filtering applied inside the FOD at build time, or the two independently-computed hashes will never agree. If `generate` and `fetch-dependency.sh` end up implemented as genuinely separate code paths (likely, since one runs outside Nix and one runs inside a derivation), this needs an explicit shared/tested filter rule, not two hand-copies that can drift.

All fetched dependencies get symlink-assembled into a single directory (mirroring `gomod2nix`'s `mkVendorEnv`), in whatever shape `gos build`'s eventual non-`path` dependency consumption expects — this shape is currently undefined upstream (the §0 gate), so this part of the design is necessarily provisional.

### 3.3 `buildGossamerApplication` integration

Extend the existing `nix/builder.nix` (currently a thin `gos build --release --out-dir dist` wrapper) to accept an optional `depsLock` attribute (path to a generated `gossamer2nix.toml`). When present, assemble the fetched-dependency tree (§3.2) and make it available to `gos build --locked` before invoking it — offline, sandboxed, exactly like every prior-art tool's final compile step. When absent (today's status quo), behave exactly as `nix/builder.nix` does now — path-only dependencies, no lock, no change to current behavior. This keeps the existing `hello-app` check (`nix/checks.nix`) working unmodified while the non-path story is built out incrementally.

---

## 4. `[rust-bindings]` (the FFI/Cargo sub-problem)

Per `DEPS.md` §3.1, `[rust-bindings]` is a second, independent dependency graph — Cargo's, not Gossamer's — with five variants (`Path`, `Git`, `Crates`, `Src`, `Prebuilt`). This needs its own answer, separate from §3's registry/git/tarball design, because Cargo's ecosystem already has two mature prior-art tools to choose between (`CRATE2NIX.md`, `NAERSK.md`).

**Recommendation: follow naersk's technique (lockfile-is-truth, no generated/checked-in Nix file), not crate2nix's (generated `Cargo.nix` codegen), for the same reason `gossamer2nix` should trust `project.lock` at the top level (§1.1).** Gossamer already has one generated-lockfile-must-stay-in-sync concern (`project.lock` itself); adding a second one for the FFI side (a `crate2nix`-style committed `Cargo.nix`) is an avoidable second drift surface for a part of the system that's explicitly secondary. `naersk`'s core mechanism — read `Cargo.lock`'s `checksum` field directly and verbatim (§2 of `NAERSK.md`, no re-hashing needed at all, unlike Gossamer's own §2 question above), fetch each crate as its own `fetchurl` FOD, and use Cargo's own `[source] replace-with` directory-source mechanism to make `cargo build` believe it already has everything — needs no codegen step and stays automatically in sync with whatever `Cargo.lock` says.

Two caveats carried over honestly from `NAERSK.md` rather than smoothed over:
- **naersk's own maintenance status is a real yellow flag** (orphaned since the original author's Dec-2023 "looking for a maintainer" post, per `NAERSK.md` §0) — secondary sources point at [`ipetkov/crane`](https://github.com/ipetkov/crane) as the community's de facto successor. Before writing code against naersk directly, it's worth a short parallel look at `crane`'s equivalent mechanism (not yet surveyed in this doc set) — the *technique* (checksum-reuse + Cargo source-replacement) is what's being adopted here, not necessarily a hard runtime dependency on the `naersk` repo itself.
- `[rust-bindings]` is inline TOML inside `project.toml`, not a standalone Cargo project — so before either tool's machinery applies at all, `gossamer2nix` (or `gos`'s own FFI scaffolding, if it already does this — unconfirmed, per `CRATE2NIX.md`'s relevance section) needs to materialize a synthetic `Cargo.toml`/`Cargo.lock` pair from the `Crates`/`Git`/`Path` entries. The `Src` variant (inline single-file binding) still routes through this same synthetic-manifest path once expanded; `Prebuilt` (archive keyed by ABI version) bypasses Cargo entirely and is just a plain content-addressed `fetchurl`, no crate-graph tooling involved.

This is lower priority than §0/§2/§3 — it only matters once a Gossamer project actually declares `[rust-bindings]`, and is architecturally independent of whether the top-level `.gos` dependency story (§3) is resolved.

---

## 5. Proposed phasing

1. **Phase 0 (now, no new code):** Status quo. `nix/builder.nix` handles path-only deps. Unaffected by anything below.
2. **Phase 1 (settle §0):** Add a second `nix/checks.nix` smoke test that gives a scaffolded project a registry dependency and observes whether `gos build --locked` does anything with it. This is cheap, empirical, and blocks every subsequent phase from being worth doing.
3. **Phase 2 (settle §2):** Once dependencies are confirmed real, compute Gossamer's native `sha256` and an independent NAR+SHA-256 for the same fetched dependency and compare. This one comparison determines whether §3.1's `generate` step needs a full refetch-and-rehash (`gomod2nix`-shaped) or a cheap reformat (`poetry2nix`-shaped).
4. **Phase 3:** Build `gossamer2nix generate` + the per-dependency FOD mechanism (§3.1–§3.2) for registry and git dependencies first (git is the easy case per §3.1 — already commit-pinned). Tarball (`url`) dependencies next, reusing whatever §2 established. Extend `buildGossamerApplication` (§3.3) behind an opt-in `depsLock` attribute so existing path-only builds are unaffected.
5. **Phase 4 (independent of 1–4, lower priority):** `[rust-bindings]` support (§4), once a real project needs it.

---

## Sources

Synthesized from this repo's own prior research:
- [`../DEPS.md`](../DEPS.md) — Gossamer's dependency model, read from `gossamer-pkg`/`gossamer-resolve`/`gossamer-cli`/`gossamer-driver` source, 2026-07-25.
- [`GOMOD2NIX.md`](./GOMOD2NIX.md), [`CRATE2NIX.md`](./CRATE2NIX.md), [`NAERSK.md`](./NAERSK.md), [`BUN2NIX.md`](./BUN2NIX.md), [`PNPM2NIX.md`](./PNPM2NIX.md), [`POETRY2NIX.md`](./POETRY2NIX.md), [`PIP2NIX.md`](./PIP2NIX.md), [`ZON2NIX.md`](./ZON2NIX.md) — eight prior-art `*2nix` tools, all read from upstream source 2026-07-26.

As with every document in this set: this reflects one point-in-time synthesis and rests on §0's gating question being unresolved as of this writing. Re-verify against current upstream `gossamer` source before treating §3's registry/git/tarball design as buildable.
