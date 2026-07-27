# Dependency Management Bridging in pnpm2nix

This document was compiled by reading source directly — not just README/marketing copy — from three points in one fork lineage: [nix-community/pnpm2nix](https://github.com/nix-community/pnpm2nix) (`master`), [nzbr/pnpm2nix-nzbr](https://github.com/nzbr/pnpm2nix-nzbr) (`main`, specifically `derivation.nix` and `lockfile.nix`), and [FliegendeWurst/pnpm2nix-nzbr](https://github.com/FliegendeWurst/pnpm2nix-nzbr) (`main`, same two files), plus GitHub's issue/PR/commit metadata for all three repos and the [pnpm.io](https://pnpm.io) docs for pnpm's own store model.
Read via GitHub's raw-content and REST APIs on **2026-07-26**; no local clone of any of these repos exists in this workspace.

It exists to ground this repo's `gossamer2nix` deps-lock/Nix adapter design (see [GOALS.md](../../GOALS.md)) in how a comparable, longer-lived `*2nix` tool actually bridges a package manager's lockfile into a sandboxed Nix build — as prior art, not as a template to imitate wholesale (pnpm's ecosystem shape — content-addressable store, symlinked `node_modules`, registry tarballs — is quite different from Gossamer's).

**This reflects one point-in-time reading of upstream `main`/`master` and may drift. Re-verify against current upstream source before depending on exact details.**

---

## 0. Which tool, and why this is a fragmented space (important finding)

There is **no single canonical, actively-maintained `pnpm2nix`.** The name has passed through an abandoned original and a popular-but-stale fork, and the actual current fixes live in a low-visibility fork of that fork:

1. **[nix-community/pnpm2nix](https://github.com/nix-community/pnpm2nix)** — the original. Its own `README.org` states in bold at the top: *"Status: Unmaintained, only compatible with lockfile version 5.0 or below (latest is 9.0 at the time of writing)"* and explicitly redirects readers elsewhere: *"A maintained fork which supports pnpm version 10.0 can be found [here](https://github.com/FliegendeWurst/pnpm2nix-nzbr)."* GitHub shows `pushed_at: 2026-04-20`, but that's dependency-bot/CI churn, not functional work — the `README.org`'s own unmaintained notice is the load-bearing signal.
2. **[nzbr/pnpm2nix-nzbr](https://github.com/nzbr/pnpm2nix-nzbr)** — the most-referenced successor (70 stars, 34 forks as of this reading, and the name most search results and other projects' flake inputs point to, e.g. it's referenced from `coder/coder` and multiple other flakes found via search). GitHub's `pushed_at: 2026-07-24` looks recent, but this is misleading: `git ls-remote` / the commits API show the actual `main` branch HEAD is commit `0366b734`, **"Ensure node modules are stored in a `node_modules` directory. Fixes #31," dated 2024-01-31** — i.e. no functional commit to `main` in roughly two and a half years as of 2026-07-26. The `2026-07-24` push activity is from Renovate-bot branches (`renovate/font-awesome`, `renovate/lock-file-maintenance`, etc.), not merged work. Its bundled example lockfile is pinned at `lockfileVersion: '6.0'` — the format generation *before* pnpm 9's lockfile restructuring (§7). Two directly relevant fixes sit unmerged: [PR #40 "Fix incompability about pnpm lockfile v9"](https://github.com/nzbr/pnpm2nix-nzbr/pull/40) (opened 2024-06-09, still open) and [PR #35 "Add support for pnpm workspaces"](https://github.com/nzbr/pnpm2nix-nzbr/pull/35) (opened 2024-03-05, still open).
3. **[FliegendeWurst/pnpm2nix-nzbr](https://github.com/FliegendeWurst/pnpm2nix-nzbr)** — a fork of a fork (`nzbr/pnpm2nix-nzbr` → `mojotech/pnpm2nix-nzbr` → this repo), only 2 stars, but the one nix-community's own unmaintained-notice points to, and the one that's actually being kept current: commits as recent as **2026-07-25** ("Fix log spam due to `pnpm store add`"), plus 2026 commits fixing `ERR_PNPM_RESOLUTION_SHAPE_MISMATCH`, git-hosted GitHub deps with an `integrity` field, and unfiltered optional dependencies. Its README states up front: *"This fork supports pnpm 10 and pnpm workspaces."*

**Conclusion / what I documented:** the *mechanism* (fetcher dispatch, patched-lockfile trick, two-stage store-then-install derivation) is common to all three and is described below primarily from `nzbr/pnpm2nix-nzbr`'s `lockfile.nix`/`derivation.nix`, since that's the version most other projects actually reference and the structure is clearest there. Where FliegendeWurst's fork has since fixed or extended something materially (lockfile v9 `snapshots` handling, workspaces, optional-dependency platform filtering), that's called out explicitly and attributed. Treat "pnpm2nix" in this doc as *this fork lineage's shared design*, not one single actively-maintained project — that fragmentation is itself a datapoint for `gossamer2nix`: a single greenfield `*2nix` implementor with no ecosystem-wide adoption pressure can stagnate quickly once the upstream package manager's lockfile format moves.

---

## 1. Input

**What ecosystem-native data goes in**, per `mkPnpmPackage` (`derivation.nix` in all three repos):

| Input | Default location | Purpose |
|---|---|---|
| `package.json` | `${src}/package.json` | `name`/`version` → derivation `pname`/`version`; `scripts.<script>` (default `build`) is what actually runs |
| `pnpm-lock.yaml` | `${src}/pnpm-lock.yaml` | the entire dependency graph + integrity data (below) |
| `registry` | `https://registry.npmjs.org` (default, overridable) | base URL used to reconstruct registry tarball download URLs |

`pnpm-lock.yaml` is parsed by shelling out to `remarshal`'s `yaml2json` inside a Nix `runCommand` (`parseLockfile = lockfile: builtins.fromJSON (readFile (runCommand "toJSON" {} "${remarshal}/bin/yaml2json ${lockfile} $out"));`) — i.e. YAML→JSON conversion happens as a derivation, then Nix's own `builtins.fromJSON` parses the result into a Nix attrset for further processing at eval time.

Per-package, the code reads (`lockfile.nix`, `findTarball`/`processLockfile`):

- The `packages` section's keys — parsed via `splitVersion`/`getVersion`/`withoutVersion` (split on `@`/`(` to strip a trailing peer-deps parenthetical like `(sharp@0.31.3)` and separate name from version; the `@`-splitting is scope-aware so `@scope/name@1.0.0` round-trips correctly).
- `resolution.type` (e.g. `"git"`), `resolution.repo` / `resolution.commit` for git-sourced packages.
- `resolution.tarball` + `resolution.integrity` for direct-tarball and (in the newer fork) `codeload.github.com` archive URLs.
- `v.id` (an alternate git-style `host/owner/repo/rev` shape seen for some GitHub dependency specs).
- `resolution.integrity` for registry packages, in **SRI format** (e.g. `sha512-<base64>`), consumed directly (§2).
- `v.dev` (boolean) — used to exclude devDependencies when `noDevDependencies = true`.
- In FliegendeWurst's fork only: `v.os` / `v.cpu` (pnpm's platform-restriction fields on optional deps) and a top-level `snapshots` section — both absent from the `lockfileVersion: '6.0'` format `nzbr`'s code was written against; see §7.

Not read/consumed by any of the three: `settings`, `specifiers`, `importers` (workspace root manifest data — until FliegendeWurst's workspace support), publish-time signature/provenance data.

---

## 2. Output

**What Nix-consumable data comes out**, per `mkPnpmPackage`'s `passthru`:

- **`dependencyTarballs`** — a deduplicated list of Nix store paths, each a `.tgz` for one locked package, produced either directly by `fetchurl` (registry/tarball case) or synthesized from a `fetchGit` checkout via a `mkTarball` helper (`tar -czf $out -C ${contents} .`) for git-sourced deps.
- **`patchedLockfile`** / **`patchedLockfileYaml`** — the original lockfile JSON with every package's `resolution.tarball` field rewritten to `file:${storePath}` pointing at the corresponding entry in `dependencyTarballs`, then serialized back out via `writeText "pnpm-lock.yaml" (toJSON ...)`. **Notable trick:** the file is named `pnpm-lock.yaml` but its *contents* are JSON — this works unmodified because JSON is a syntactic subset of YAML, so pnpm's YAML parser reads it correctly. This sidesteps writing a real YAML serializer in Nix.
- **`pnpmStore`** — a derivation that pre-populates a pnpm content-addressable store (§4) from `dependencyTarballs`, entirely offline from Nix's perspective (the network fetches already happened as separate fixed-output derivations).
- **`nodeModules`** — a derivation that runs `pnpm install --frozen-lockfile --offline` (optionally `--prod`) against the patched lockfile and pre-populated store, producing a real, symlinked `node_modules` tree with no network access during the Nix build itself.
- The final package derivation runs `pnpm run <script>` against that `node_modules` and installs `${distDir}` (default `dist`) as `$out`.

### Hash format: SRI reused verbatim, not re-derived

This is the single most important input→output mechanism (§2 of the task brief). For both registry and tarball+integrity cases, `lockfile.nix` does:

```nix
fetchurl {
  url = v.resolution.tarball;  # or the reconstructed registry URL
  ${head (splitString "-" v.resolution.integrity)} = v.resolution.integrity;
};
```

`v.resolution.integrity` is pnpm's own SRI string, e.g. `sha512-oIQMi6ovbnpvz2zGjWGKUUKm5+Kc/HpDsCxJkNb8x4l...`. `head (splitString "-" ...)` extracts the algorithm name (`sha512`) and uses it as the *attribute name* passed to `fetchurl`, with the **full original SRI string** (algorithm prefix included) as the value. Nix's fetchers recognize an SRI-prefixed string as a hash value regardless of which legacy per-algorithm attribute name it's assigned to, so this reuses pnpm's own integrity hash unmodified as the Nix fixed-output-derivation hash — no re-hashing, no format conversion, no separate hash database. This is the direct pnpm-lock.yaml-integrity → Nix-FOD-hash bridge the task asked about, and it's about as thin as such a bridge can be.

Where a lockfile entry has *no* integrity field — pnpm itself doesn't always provide one for `resolution: tarball` deps pointing at non-npm-registry URLs — the fetch cannot be made a pure fixed-output derivation from lockfile data alone; see [nzbr issue #13](https://github.com/nzbr/pnpm2nix-nzbr/issues/13) (§5).

---

## 3. Why this tool exists: the pnpm/Nix gap

`pnpm install` wants to talk to a registry (or clone git repos, or fetch tarball URLs) over the network and write into a shared, machine-global content-addressable store outside the project directory. A Nix build derivation has neither: no network access during the build phase (aside from designated fixed-output derivations), and no shared mutable state across builds — every derivation gets a fresh, isolated `$TMPDIR`/`$HOME`.

pnpm's store model is a specific complication relative to npm/yarn here, in both directions:

- **Easier**, in one sense: pnpm's lockfile already carries a **content-addressable integrity hash per package** (`resolution.integrity`, SRI format) that maps directly onto what a Nix fixed-output derivation needs. `package-lock.json` (npm) and `yarn.lock` also carry integrity hashes, so this isn't unique to pnpm, but pnpm's lockfile is comparatively uniform about it per-package.
- **Harder**, in another sense: pnpm's `node_modules` is not simply "download tarballs, unpack side by side" (npm/yarn-classic's flatter, hoisted model). Per [pnpm's own docs](https://pnpm.io/symlinked-node-modules-structure), every file of every installed package is a **hard link into a single content-addressable store**, and the `node_modules` tree pnpm actually builds is a graph of **symlinks** — a nested `node_modules/.pnpm/<name>@<version>/node_modules/<name>` per-package directory hard-linked from the store, with the project's own `node_modules/<name>` and each package's own `node_modules/<dep>` being symlinks into that `.pnpm` virtual store, mirroring the real (non-hoisted, non-flat) dependency graph rather than flattening it. Because pnpm's own `install` step is what constructs this symlink graph — not a step this tooling reimplements — pnpm2nix doesn't need to hand-replicate `.pnpm`'s internal layout; it only needs to make `pnpm install --offline` succeed inside the sandbox, and let pnpm build its own store/symlink structure from content it already trusts.

The actual bridge, concretely:

1. Fetch every locked package as a plain tarball via ordinary Nix fixed-output derivations (`fetchurl`/`fetchGit` + the SRI hash straight from the lockfile, §2) — this is the *only* step that touches the network, and it does so outside the sandboxed build.
2. In a second, network-free derivation (`pnpmStore`), point `pnpm store path` at a fresh `$out` (via a symlink) and run `pnpm store add <tarball paths...>`, which pnpm accepts as local file arguments and content-hashes into its own store layout — this reconstructs pnpm's expected on-disk content-addressable store from already-fetched, already-verified tarballs, with no network call.
3. In a third derivation (`nodeModules`), symlink or copy that populated store into place, supply the **patched lockfile** — whose `resolution.tarball` fields now point at `file:` URIs already sitting in the Nix store rather than the original network URLs — and run `pnpm install --frozen-lockfile --offline`. Because the lockfile is otherwise byte-for-byte the original resolution graph, `--frozen-lockfile` is satisfied, and because every tarball pnpm would want is already store-resident with matching content, `--offline` succeeds. pnpm itself then builds the real `.pnpm`/symlink `node_modules` structure, entirely inside the sandbox.
4. The actual package build (`configurePhase` links or copies that `node_modules` in; `buildPhase` runs `pnpm run <script>`) is then a normal, offline `stdenv.mkDerivation` build.

So the "gap" pnpm2nix bridges is specifically: *turning a lockfile's declared, content-addressed dependency graph into pre-fetched, pre-verified tarballs (network-permitted step), then handing those to pnpm's own installer running fully offline inside the sandbox (network-forbidden step), rather than trying to reimplement pnpm's store/symlink logic in Nix.* It leans on pnpm's own `store add`/`install --offline` machinery rather than replicating `.pnpm`'s internal directory format by hand.

---

## 4. The store/`node_modules` mechanism in more detail

Two nested derivations do the real work (`derivation.nix`, `passthru`):

**`pnpmStore`** (`runCommand`):
```nix
mkdir -p $out
store=$(pnpm store path)
mkdir -p $(dirname $store)
ln -s $out $(pnpm store path)
pnpm store add ${concatStringsSep " " (unique processResult.dependencyTarballs)}
```
This makes `$out` *be* pnpm's content-addressable store directory (by symlinking pnpm's expected store path to `$out` before running `pnpm store add`), then hands `pnpm store add` every fetched tarball's Nix store path as a plain filesystem argument. `store add` is pnpm's own command for importing already-downloaded package archives into its CAS by content hash — no registry contact needed since the argument is a local path.

**`nodeModules`** (`stdenv.mkDerivation`):
- `unpackPhase` copies in `package.json` and the *patched* `pnpm-lock.yaml` (plus any `extraNodeModuleSources`).
- `buildPhase` re-points `pnpm store path` at the `pnpmStore` derivation's output (`ln -s` by default; `cp -RL` + `chmod -R +w` if `copyPnpmStore` — which defaults to `true` — is set, apparently to dodge an `EACCES` error copying from a read-only symlinked store, per an inline comment), then runs `pnpm install --frozen-lockfile [--prod] --offline`.
- `installPhase` copies the resulting `node_modules` out as the derivation's `$out`.

The top-level `mkPnpmPackage` derivation then either **symlinks** (`ln -s`, the default) or **copies** (`copyNodeModules = true`) that `node_modules` output into its own build directory before running `pnpm run <script>` — symlinking is cheaper but means the final build derivation depends on (and can't modify) the `nodeModules` derivation's output in place.

---

## 5. Limitations and gotchas (as documented in issues/PRs, not just inferred)

- **Lockfile format drift (`nzbr`'s main branch specifically):** targets `lockfileVersion: '6.0'`-era `packages` keys (leading `/`, e.g. `/lodash@4.17.21`, with an optional `(peer@version)` suffix). pnpm 9's lockfile restructuring split this into a deduplicated `packages` section (keys without the leading `/`) plus a separate `snapshots` section carrying the per-peer-permutation dependency edges. [PR #40](https://github.com/nzbr/pnpm2nix-nzbr/pull/40) ("Fix incompability about pnpm lockfile v9") has sat open since 2024-06-09 on `nzbr/pnpm2nix-nzbr`; FliegendeWurst's fork has since added explicit `snapshots`-aware filtering (visible in its `lockfile.nix`'s `filteredSnapshots`/`filterDeps` logic) — confirming the base fork genuinely can't read a current pnpm-generated lockfile without that patch. Exact `packages`/`snapshots` key-format details are inferred from reading FliegendeWurst's code, not pnpm's own format spec — **treat the precise key grammar as unconfirmed.**
- **Monorepo/workspace support:** not present on `nzbr/pnpm2nix-nzbr`'s `main` — [issue #29](https://github.com/nzbr/pnpm2nix-nzbr/issues/29) ("Are pnpm workspaces supported?") was closed without a fix, and [PR #35](https://github.com/nzbr/pnpm2nix-nzbr/pull/35) adding `workspace`/`components` arguments has been open since 2024-03-05 and unmerged. FliegendeWurst's fork adds `workspace`, `components`, and `pnpmWorkspaceYaml` parameters and its README claims workspace support; not independently verified end-to-end in this reading.
- **Tarball dependencies with no integrity hash:** [issue #13](https://github.com/nzbr/pnpm2nix-nzbr/issues/13) documents that pnpm doesn't always emit an `integrity` field for `resolution: tarball`-kind dependencies (cited real-world case: Misskey's non-npm-registry deps), which breaks the "reuse SRI hash directly" mechanism (§2) since there's nothing to reuse; the reporter proposed (and, per the issue, intended to fork to add) a manual dependency-override escape hatch. Not confirmed whether this landed in either fork's `main`.
- **Native/postinstall build scripts:** [issue #36](https://github.com/nzbr/pnpm2nix-nzbr/issues/36) shows a real failure building `better-sqlite3` (a native addon with an `install`-time compile step) under `mkPnpmPackage` — native-module postinstall/build steps are a known rough edge, unsurprising given the sandbox has no network and only whatever's in `nativeBuildInputs`/`extraBuildInputs`.
- **Peer dependency resolution:** not separately validated by this tool — it passes through whatever pnpm already resolved into the lockfile (including peer-dependency-driven duplicate `(peer@version)`-suffixed package instances) rather than re-deriving or re-checking peer compatibility itself. FliegendeWurst's fork additionally does its own `os`/`cpu` platform filtering of optional dependencies (`isPlatformCompatible`) to avoid trying to fetch e.g. Windows-only binaries when building on Linux — a gap the base fork doesn't handle, per its 2026-03-22 commit message ("fix: optional dependencies not filtered").
- **Patched dependencies** (pnpm's `patchedDependencies` / `pnpm patch` feature): no handling found in any of the three `lockfile.nix` readings; not confirmed either way beyond "no code path was found," so treat as **unconfirmed/likely unsupported** rather than assumed absent.
- **IFD (import-from-derivation) cost:** FliegendeWurst's README flags this directly: *"The supplied `pnpmLockYaml` is processed using a lot of (slow) IFD logic. To cache those results more efficiently, pass it explicitly as `pnpmLockYaml = ./pnpm-lock.yaml`."* — i.e. lockfile parsing itself happens via a derivation evaluated during Nix evaluation (not just build), which is inherently slower and less cacheable than pure-Nix parsing would be.

---

## 6. Comparison note for `gossamer2nix`

The most directly transferable idea, independent of pnpm-specific mechanics: **separate "fetch" (network-permitted, one fixed-output derivation per content-addressed unit) from "assemble" (network-free, ordinary derivation that only trusts hashes already verified in step one).** Gossamer's own lockfile (`project.lock`, see [DEPS.md §7](../DEPS.md)) already records a `sha256` per non-path dependency that is a content-addressed digest of the fetched, verified source tree — structurally the same shape as pnpm's per-package `resolution.integrity`, and reusable the same way pnpm2nix reuses SRI hashes directly as Nix FOD hashes (§2 above), without needing a translation table.

Where the analogy breaks down: pnpm2nix's hard part is reconstructing an ecosystem-specific *runtime* artifact (pnpm's own CAS + symlinked `node_modules`) inside the sandbox by shelling back out to the ecosystem's own tool (`pnpm store add` / `pnpm install --offline`) — it does not reimplement pnpm's store logic in Nix, it drives pnpm itself, offline, against pre-staged content. Per [DEPS.md §12](../DEPS.md), it's still unconfirmed whether `gos build` has *any* consumption path for non-path dependencies at all (registry/git/tarball), which is a prerequisite question pnpm2nix never had to face — pnpm's `install` step reliably and unambiguously wires fetched packages into a build-visible `node_modules`. A `gossamer2nix` design can't productively borrow pnpm2nix's "drive the ecosystem tool offline" pattern until that gap in `gos` itself is resolved one way or the other.

---

## Sources

- [nix-community/pnpm2nix](https://github.com/nix-community/pnpm2nix) — `README.org` (unmaintained notice, fork pointer), `default.nix`, repo metadata via `gh api repos/nix-community/pnpm2nix`.
- [nzbr/pnpm2nix-nzbr](https://github.com/nzbr/pnpm2nix-nzbr) — `README.md`, `derivation.nix`, `lockfile.nix`, `flake.nix`, `example/{package.json,pnpm-lock.yaml}`, repo metadata and commit history via `gh api repos/nzbr/pnpm2nix-nzbr{,/commits,/branches}`, and issues/PRs [#13](https://github.com/nzbr/pnpm2nix-nzbr/issues/13), [#29](https://github.com/nzbr/pnpm2nix-nzbr/issues/29), [#35](https://github.com/nzbr/pnpm2nix-nzbr/pull/35), [#36](https://github.com/nzbr/pnpm2nix-nzbr/issues/36), [#40](https://github.com/nzbr/pnpm2nix-nzbr/pull/40).
- [FliegendeWurst/pnpm2nix-nzbr](https://github.com/FliegendeWurst/pnpm2nix-nzbr) — `README.md`, `lockfile.nix`, repo metadata and commit history via `gh api repos/FliegendeWurst/pnpm2nix-nzbr{,/commits}`.
- [mojotech/pnpm2nix-nzbr](https://github.com/mojotech/pnpm2nix-nzbr) — fork-ancestry check only (`gh api repos/mojotech/pnpm2nix-nzbr`).
- [pnpm.io — Symlinked `node_modules` structure](https://pnpm.io/symlinked-node-modules-structure) — pnpm's own content-addressable store / `.pnpm` virtual store model.
- [pnpm/pnpm v9.0.0 release notes](https://github.com/pnpm/pnpm/releases/tag/v9.0.0) — confirms lockfile v9 format change ("better readability, better resistance to Git conflicts") at a high level; exact key-format details for `packages`/`snapshots` in this doc are inferred from reading FliegendeWurst's `lockfile.nix` handling of both sections rather than from pnpm's own format spec, and should be treated accordingly (**unconfirmed against pnpm's own docs**).

Read on 2026-07-26. As with `DEPS.md`, this is one point-in-time reading; re-verify commit dates, open/closed issue status, and lockfile-format specifics against current upstream before depending on exact details in code.
