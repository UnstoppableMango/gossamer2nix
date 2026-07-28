# Dependency Management Bridging in `bun2nix`

Read 2026-07-26 (`master`, commit `0f2a1f0b6f42ce`, tag `2.1.2`) as prior art for `gossamer2nix`'s own deps-lock/Nix adapter. Sources: `programs/bun2nix/src/**` (Rust CLI), `programs/cache-entry-creator/src/main.zig`, `nix/**`, `docs/src/**`.

Multiple projects share the name `bun2nix`; this document covers `nix-community/bun2nix` (formerly `baileyluTCD/bun2nix` — confirmed a repo-transfer redirect, same history, same maintainer), chosen as canonical since it's the redirect target, lives under `nix-community`, and has active CI/docs. Distinct, undocumented projects with the same name: `aabccd021/bun2nix-rs`, `aabccd021/bun3nix`, `nyadiia/bun2nix`.

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/bun2nix](https://github.com/nix-community/bun2nix) (`master`) |
| **Ecosystem** | Bun (JS runtime + package manager) |
| **Maintenance status** | Active |
| **Ecosystem input** | `bun.lock` — JSON/JSONC textual lockfile only. **The binary `bun.lockb` format is never read**, a hard scope cut, not a partial gap |
| **Generated output** | `bun.nix` — a Nix function from fetcher primitives to one fetcher call per lockfile package |
| **Hash strategy** | Hybrid — npm packages' `sha512-` SRI integrity hash reused verbatim; git/GitHub/tarball deps (no hash in `bun.lock`) re-hashed via `nix flake prefetch` at generation time |
| **Nix build mechanism** | Per-package FODs reassembled into a symlink farm that mimics Bun's own global install cache, consumed by an unmodified `bun install` |

## Input

`bun.lock` is parsed as JSONC (`jsonc_parser` crate), requiring Bun ≥ 1.2 — the version that made this the default lockfile format. Top-level fields read: `lockfileVersion`, `workspaces`, and `packages` — a map from package id to a **tuple whose arity determines its kind**:

| Arity | Kind | Notes |
|---|---|---|
| 1 | Workspace | local monorepo member |
| 2 | File / local tarball | `file:` deps |
| 3 | Git / GitHub / remote tarball | dispatched by URL prefix |
| 4 | npm registry | `[name@version, tarballUrl, metadata, integrityHash]` |

For workspaces, any dependency literally pinned to `"latest"` triggers a warning that reproducibility may not hold. `catalog:` shared-version specifiers (Bun's pnpm-style catalogs) are **not** parsed by the Rust tool at all — they're rewritten entirely on the Nix/shell side at build time (see Limitations).

## Output and hash handling

```nix
"@types/bun@1.2.4" = fetchurl {
  url = "https://registry.npmjs.org/@types/bun/-/bun-1.2.4.tgz";
  hash = "sha512-QtuV5OMR8/rdKJs213iwXDpfVvnskPXY/S0ZiFbsTjQZycuqPbMW8Gf/XhLfwE5njW8sxI2WjISURXPlHypMFA==";
};
```

npm packages' lockfile integrity hash is written verbatim — no re-hashing, no network call at generation time; Nix's `fetchurl` natively accepts SRI. Git/GitHub/plain-tarball deps carry **no** integrity hash in `bun.lock` at all, so `bun2nix` shells out to `nix flake prefetch <url> --json` at CLI-run time to mint one — meaning generating `bun.nix` for such a lockfile requires network access and a working `nix` binary on the generating machine (the WASM/npm-distributed CLI, `bunx bun2nix`, can't do this at all and errors out). File/workspace-local packages get no fetch or hash — just `copyPathToStore`. Notably, npm packages use `pkgs.fetchurl` rather than `builtins.fetchTarball`, deliberately, because `fetchTarball`'s hash covers the *extracted* tree, which wouldn't match the tarball-level integrity hash `bun.lock` records.

## Nix-side mechanism: rebuilding Bun's own cache

`bun2nix` doesn't try to make `bun install` run against a rewritten `node_modules`. It reconstructs Bun's **global install cache** directory structure entirely inside the Nix store, then points `bun install` at it:

1. **Fetch**: evaluate `bun.nix` → one FOD (or store-copy) per package.
2. **Extract + normalize**: unpack with `bsdtar`, patch shebangs/ELF binaries as needed.
3. **Re-key into Bun's cache-directory naming scheme** — the crux. Bun's cache key for a package isn't just `name@version`; for prerelease versions it's hashed with Bun's own vendored Wyhash implementation. `bun2nix` **reimplements this exact algorithm in Zig** (with unit tests asserting bit-for-bit agreement with real Bun cache folder names), because a wrong key means Bun's installer won't recognize the entry and will hit the network. This is why part of the tool is Zig, not Rust.
4. **Symlink farm**: each normalized package is symlinked into `$out/share/bun-cache/<bun-cache-key>`.
5. **Force Bun to use it**: a Nix setup hook copies that cache into a fresh temp dir, exports `BUN_INSTALL_CACHE_DIR`, then runs a completely ordinary `bun install --linker=isolated --ignore-scripts` inside the sandbox — Bun believes it already has a warm cache and never touches the network.

## Why the tool exists

`bun install` wants network access to an npm-compatible registry; Nix sandboxes builds. `bun2nix`'s answer: shift all network fetching into per-package FODs (hash-verified, reproducible) ahead of time, then make `bun install` believe it's a warm-cache no-op by faithfully reconstructing Bun's own cache layout. `bun.lock` is the single source of truth for exactly which package/version/hash to fetch.

## Limitations

- **`catalog:` specifiers**: Bun re-resolves these against the npm registry on every `install`, even with a warm cache, so a helper script rewrites every reference to its resolved version in the lockfile/`package.json` *before* `bun install` runs.
- **`patchedDependencies`**: stripped before install; patches are applied ahead of time to the Nix-store package derivation instead, with cache-key suffixes deliberately avoided so lookups still hit the un-suffixed key.
- **Non-default/private registries**: supported via an explicit `tarballUrl` per package plus `bunfig.toml`/`.npmrc` credential parsing wrapping `fetchurl` with an `Authorization` header.
- **`bun.nix`'s schema is not guaranteed stable** across `bun2nix` versions — treated as a disposable build artifact, not something meant for long-term diffing.
- Optional/platform-specific dependency handling: not confirmed either way in the source read.
