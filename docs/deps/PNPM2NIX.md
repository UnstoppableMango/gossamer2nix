# Dependency Management Bridging in `pnpm2nix`

Read 2026-07-26 as prior art for `gossamer2nix`'s deps-lock/Nix adapter design. Sources: three points in one fork lineage — [nix-community/pnpm2nix](https://github.com/nix-community/pnpm2nix), [nzbr/pnpm2nix-nzbr](https://github.com/nzbr/pnpm2nix-nzbr) (`derivation.nix`, `lockfile.nix`), [FliegendeWurst/pnpm2nix-nzbr](https://github.com/FliegendeWurst/pnpm2nix-nzbr) (same two files) — plus [pnpm.io](https://pnpm.io)'s docs on pnpm's own store model.

**This is a genuinely fragmented space, itself a finding worth carrying into `gossamer2nix`'s risk assessment.** `nix-community/pnpm2nix` (the original) declares itself unmaintained in its own README, capped at lockfile v5 (current is v9+), and points readers at `FliegendeWurst/pnpm2nix-nzbr`. The most-referenced successor, `nzbr/pnpm2nix-nzbr` (70 stars), looks recently active by GitHub's `pushed_at` metadata, but that's Renovate-bot branch noise — its `main` branch's last functional commit was 2024-01-31, and two directly relevant fixes (pnpm lockfile v9 support, workspace support) have sat unmerged as open PRs since 2024. The fork that's actually current is the low-visibility `FliegendeWurst/pnpm2nix-nzbr` (2 stars, commits as recent as 2026-07-25). The mechanism below is described from `nzbr`'s clearer source (what most other projects still reference); where FliegendeWurst's fork fixes something material, that's called out explicitly.

## At a glance

| | |
|---|---|
| **Canonical repo** | No single one — [FliegendeWurst/pnpm2nix-nzbr](https://github.com/FliegendeWurst/pnpm2nix-nzbr) is the actually-maintained fork of a fork |
| **Ecosystem** | pnpm (JS) |
| **Maintenance status** | Fragmented — original unmaintained, popular fork stale since 2024-01, real fixes live in a low-visibility third fork |
| **Ecosystem input** | `pnpm-lock.yaml` (per-package `resolution.integrity`, `resolution.type`/`repo`/`commit`/`tarball`) |
| **Generated output** | No single named file — a `patchedLockfile`, `dependencyTarballs`, `pnpmStore`, and `nodeModules` derivation chain |
| **Hash strategy** | Reused verbatim — the lockfile's own SRI `resolution.integrity` string is passed straight through as the `fetchurl` hash |
| **Nix build mechanism** | Fetch tarballs as FODs → network-free `pnpm store add` → patched lockfile + `pnpm install --frozen-lockfile --offline` |

## Input

Per `mkPnpmPackage` (`derivation.nix`): `package.json` (`name`/`version`, and the `scripts.<script>` that actually runs, default `build`), `pnpm-lock.yaml` (the full dependency graph), and a `registry` base URL (default `registry.npmjs.org`) for reconstructing tarball download URLs. `pnpm-lock.yaml` is parsed by shelling YAML→JSON via `remarshal` inside a `runCommand`, then read with `builtins.fromJSON`. Per package, the code reads: the `packages` key (name+version, peer-deps-aware), `resolution.type`/`repo`/`commit` for git sources, `resolution.tarball`/`resolution.integrity` for tarball/registry sources, and `v.dev` to optionally exclude devDependencies. FliegendeWurst's fork additionally reads `v.os`/`v.cpu` (platform-restriction fields) and a top-level `snapshots` section — both absent from the older lockfile format `nzbr`'s code targets (see Limitations).

## Output and hash format

```nix
fetchurl {
  url = v.resolution.tarball;
  ${head (splitString "-" v.resolution.integrity)} = v.resolution.integrity;
};
```

The algorithm name is extracted from the SRI string just to pick the attribute name Nix expects; the **full original SRI string** is passed as the value. Nix's fetchers recognize an SRI-prefixed hash regardless of which legacy attribute name it's assigned to, so this reuses pnpm's own integrity hash completely unmodified — no re-hashing, no format conversion. Where a lockfile entry has no integrity field at all (pnpm doesn't always emit one for non-registry tarball deps), the fetch can't be made a pure FOD from lockfile data alone — a documented real-world gap (see Limitations).

The lockfile itself is rewritten (`patchedLockfile`): every package's `resolution.tarball` is replaced with `file:${storePath}` pointing at the already-fetched Nix store path. The result is serialized back out under the filename `pnpm-lock.yaml` but with **JSON contents** — this works unmodified because JSON is a syntactic subset of YAML, sidestepping the need for a real YAML serializer in Nix.

## Nix-side mechanism

Two nested derivations do the real work. **`pnpmStore`**: symlinks pnpm's expected store path to `$out`, then runs `pnpm store add <tarball paths>` — pnpm's own command for importing already-downloaded archives into its content-addressable store by content hash, no registry contact needed since the arguments are local paths. **`nodeModules`**: points `pnpm store path` at that populated store, copies in `package.json` + the patched lockfile, and runs `pnpm install --frozen-lockfile --offline`. Because the lockfile's resolution graph is otherwise byte-identical to the original, `--frozen-lockfile` is satisfied; because every tarball pnpm wants is already store-resident with matching content, `--offline` succeeds. pnpm then builds its own real `.pnpm`/symlink `node_modules` structure entirely inside the sandbox — pnpm2nix never reimplements that structure itself.

## Why the tool exists

`pnpm install` wants network access to a registry and writes into a shared, machine-global content-addressable store; a Nix derivation has neither. pnpm's lockfile happens to make this comparatively easy in one respect — a content-addressable integrity hash per package that maps directly onto what a Nix FOD needs — but harder in another: `node_modules` isn't a flat unpack, it's a graph of symlinks into a nested `.pnpm` virtual store that pnpm's own `install` step constructs. So pnpm2nix doesn't hand-replicate that layout; it only needs `pnpm install --offline` to succeed inside the sandbox by pre-staging content pnpm already trusts, then lets pnpm build its own structure from it.

## Limitations

- **Lockfile format drift**: `nzbr`'s `main` targets pre-v9 `packages` keys (leading `/`, e.g. `/lodash@4.17.21`); pnpm 9 split this into a deduplicated `packages` section plus a separate `snapshots` table for per-peer-permutation edges. The relevant fix PR sat open since 2024-06; FliegendeWurst's fork has since added `snapshots`-aware filtering.
- **Monorepo/workspace support**: absent on `nzbr`'s `main` (issue closed without a fix, PR unmerged since 2024-03); present in FliegendeWurst's fork, not independently verified end-to-end.
- **Tarball deps with no integrity hash**: pnpm doesn't always emit one for non-npm-registry tarball resolutions, breaking the verbatim-reuse mechanism — a documented real-world failure case with no confirmed fix landed in either fork.
- **Native/postinstall build scripts**: a documented real failure building `better-sqlite3` (native addon with an install-time compile step) — the sandbox has no network and only whatever's in `nativeBuildInputs`.
- **Peer dependency resolution**: not separately validated — passes through whatever pnpm already resolved into the lockfile.
- **Patched dependencies** (`pnpm patch`): no handling found in any of the three forks' source — unconfirmed/likely unsupported.
- **IFD cost**: lockfile parsing itself runs via a derivation evaluated during Nix evaluation, not pure Nix — FliegendeWurst's README flags this as measurably slow.

## Relevance to `gossamer2nix`

The transferable idea, independent of pnpm-specific mechanics: separate "fetch" (network-permitted, one FOD per content-addressed unit) from "assemble" (network-free, trusts hashes already verified in step one). Gossamer's own `project.lock` already records a `sha256` per non-path dependency — a content-addressed digest of the fetched, verified source tree, structurally the same shape as pnpm's `resolution.integrity`, reusable the same direct way. Where the analogy breaks: pnpm2nix's hard part is reconstructing pnpm's own runtime artifact (its CAS + symlinked `node_modules`) by driving pnpm's own tooling offline, not reimplementing its store logic in Nix — and per `../DEPS.md` §12, it's still unconfirmed whether `gos build` has *any* consumption path for non-path dependencies at all, a prerequisite question pnpm's reliable `install`-wires-into-`node_modules` behavior never had to face.
