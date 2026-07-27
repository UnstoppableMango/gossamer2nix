# Dependency Management Bridging in `bun2nix`

This document was compiled by cloning [nix-community/bun2nix](https://github.com/nix-community/bun2nix) (`master`, commit `0f2a1f0b6f42cebe3b149bf62d38754c5e0e9729`, dated 2026-07-21; tag `2.1.2`) and reading its actual source — the Rust CLI/library (`programs/bun2nix/src/**`), the Zig helper binary (`programs/cache-entry-creator/src/main.zig`), the Nix-side flake-parts modules (`nix/**`), and the shipped documentation (`docs/src/**`) and example templates (`templates/**`) — not just the README.
It exists as prior art for this repo's own `gossamer2nix` deps-lock/Nix adapter (see [GOALS.md](../../GOALS.md) and [DEPS.md](../DEPS.md)), read in the same spirit: ground design decisions in a real implementation instead of assumptions.

**This reflects one point-in-time reading of upstream `master` as of 2026-07-26 and may drift.** Re-verify against current upstream before depending on exact details.

---

## 0. Which `bun2nix`? (disambiguation)

The prompt that produced this document flagged that multiple projects are named `bun2nix`. Confirmed by fetching `https://github.com/baileyluTCD/bun2nix`, which returns an HTTP 301 redirect to `https://github.com/nix-community/bun2nix` (verified via `curl -sI`, `location:` header, 2026-07-26) — this is a GitHub repo-transfer redirect, not a fork: same repo, same star/fork counts, same 519-commit history, moved into the `nix-community` org with the original author (`@baileylu121`) still listed as maintainer (`[maintainer=@baileylu121]` in the repo description).

**This document is about `nix-community/bun2nix`** (formerly `baileyluTCD/bun2nix`). It is the tool referred to on its docs site as "Bun2Nix", packaged additionally as an npm/WASM CLI (`bunx bun2nix`, see `.github/workflows/npm-packages-publish.yml` and `programs/bun2nix/index.ts`), with a companion docs book at <https://nix-community.github.io/bun2nix/> (built via `docs/book.toml`, an mdBook).

**Other, distinct `bun2nix`-named projects found during search, not documented here:**

- [`aabccd021/bun2nix-rs`](https://github.com/aabccd021/bun2nix-rs) — a separate Rust reimplementation by a different author.
- [`aabccd021/bun3nix`](https://github.com/aabccd021/bun3nix) and the NixOS Discourse post ["Bun3nix – bun.lock to nix, with TailwindCSS plugin support"](https://discourse.nixos.org/t/bun3nix-bun-lock-to-nix-with-tailwindcss-plugins-support/69939) — explicitly created as an alternative because, per the discourse post, its author "wanted to use bun2nix" but it didn't support GitHub dependencies at the time and the relevant issue had stalled. (nix-community/bun2nix now supports GitHub deps — see §1 below — so this gap may since be closed; not independently re-verified against bun3nix.)
- [`nyadiia/bun2nix`](https://github.com/nyadiia/bun2nix) — another distinct project, not investigated further.

`nix-community/bun2nix` was chosen as canonical because (a) it is the redirect target of the original/oldest-named repo, (b) it lives under the community-governed `nix-community` GitHub org (the same org that hosts `poetry2nix`, `naersk`, etc. — tools this repo already documents), (c) it has an active docs site, CI (Hercules CI badge), and a 2026-07-21 commit as of this writing, and (d) it was the first and most consistently surfaced result across multiple search queries.

---

## 1. What input does it consume?

**Only the textual `bun.lock` (JSON/JSONC) format — never the legacy binary `bun.lockb`.**

- `programs/bun2nix/src/main.rs`: the CLI's `--lock-file` flag defaults to `./bun.lock`.
- `programs/bun2nix/src/lockfile.rs`: `Lockfile::parse_to_value` parses the file with the `jsonc_parser` crate (`jsonc_parser::parse_to_serde_value`), i.e. JSON-with-comments, matching Bun's own lockfile format (linked in-source to [oven-sh/bun#11863](https://github.com/oven-sh/bun/issues/11863), the issue that introduced the text lockfile).
- No code path in the crate references `bun.lockb` (`grep -ri lockb` over the full checkout returns nothing). The tool requires Bun ≥ 1.2 (stated in the crate's `about` string and the docs site tagline) — this is the version that made `bun.lock` the default lockfile format.
- `Error::ParseJsonc` / `Error::ReadLockfileError` messages explicitly tell users to regenerate a `bun.lock` with `bun install` if parsing fails — reinforcing that the binary format is simply out of scope, not merely unsupported-with-a-workaround.

### Lockfile shape actually parsed (`programs/bun2nix/src/lockfile.rs`, `.../package_deserializer.rs`)

Top-level fields read: `lockfileVersion` (u8), `workspaces` (map of workspace name → `{ name, dependencies, devDependencies }`), and `packages` — a map from package identifier to a **tuple encoded as a JSON array**, whose *arity* (not a type tag) determines how it's interpreted (`PackageDeserializer::deserialize_package`, `programs/bun2nix/src/lockfile/package_deserializer.rs:29-40`):

| Arity | Kind | Shape | Example |
|---|---|---|---|
| 1 | Workspace | `[id]`, id contains `workspace:<path>` | local monorepo member |
| 2 | File / local tarball | `[id, meta]`, path after `@` starts with `file:`/`./`, or a bare `http` tarball URL | `file:` deps, local `.tgz` |
| 3 | Git / GitHub / remote tarball | `[id, meta, ...]` — specifier after `@` is dispatched by prefix: `http*` → tarball, `github:*` → GitHub, else → generic git | `git+https://...#<rev>`, `github:owner/repo#<rev>` |
| 4 | npm registry package | `[identifier, tarballUrl, metadata, integrityHash]` | `"@types/bun@1.2.4"` |

For npm packages specifically (arity 4), the fields read are: `identifier` (`name@version`), `tarballUrl` (empty string for the default registry, or an explicit URL for scoped/non-default registries — e.g. GitHub Packages), a `metadata` object (dependencies, `bin`, etc. — read for pass-through but not deeply modeled), and the **integrity hash**, asserted (`debug_assert!`) to be an SRI string containing `sha512-`.

**Workspaces/monorepos**: the lockfile's `workspaces` map is read (`Lockfile::has_workspaces`), and each workspace's `dependencies`/`devDependencies` values are scanned for the literal string `"latest"`, which triggers a `warn!` log telling the user their lockfile has an *unlocked* dependency and reproducibility may not hold (`Workspace::deserialize_dependencies`, `lockfile.rs:158-189`). This is read-only awareness, not resolution — actual dependency handling per workspace member happens at the Nix-consumption layer (§3), where each workspace member becomes its own `copyPathToStore` entry (see `templates/workspace/bun.nix`, `templates/catalog/bun.nix`).

**`catalog:` dependency specifiers** (Bun's pnpm-style shared-version catalogs) are *not* parsed/rewritten by the Rust `bun2nix` tool at all. Instead, they are handled entirely on the Nix/shell side — see §5 (Gotchas) below; this is the closest analogue to the "gap between lockfile and actual build-time consumption" theme from this repo's `DEPS.md` §12.

---

## 2. What does it produce?

The CLI (`bun2nix -o bun.nix`) renders a single Nix file (or stdout) via Askama templates (`programs/bun2nix/templates/*.nix_template`, driven by `NixExpression` in `nix_expression.rs`). The output is a **function from Nix fetcher primitives to an attribute set** — i.e. `pkgs.callPackage`-compatible, not raw data:

```nix
# bun.nix, autogenerated
{ copyPathToStore, fetchFromGitHub, fetchgit, fetchurl, ... }:
{
  "@types/bun@1.2.4" = fetchurl {
    url = "https://registry.npmjs.org/@types/bun/-/bun-1.2.4.tgz";
    hash = "sha512-QtuV5OMR8/rdKJs213iwXDpfVvnskPXY/S0ZiFbsTjQZycuqPbMW8Gf/XhLfwE5njW8sxI2WjISURXPlHypMFA==";
  };
  "git:ee100d81f12ae315a81c2a664979a6cc1bce99a2" = fetchgit {
    url = "https://gitlab.com/gitlab-examples/semantic-release-npm";
    rev = "ee100d81f12ae315a81c2a664979a6cc1bce99a2";
    hash = "sha256-jHz3ybhO4oQVk7sKkMpbKtanZnR3eetiUytARuy2mJM=";
  };
}
```

(Real excerpts from `templates/git-deps/bun.nix` and `templates/default/bun.nix` in the checkout.)

### Hash handling: reused directly for npm, re-hashed (via `nix flake prefetch`) for everything else

This is the single most important mechanical fact for a `*2nix` adapter to internalize, so it's worth stating precisely (`programs/bun2nix/src/package/fetcher.rs`):

- **npm registry packages (arity-4 entries)**: the lockfile's own integrity hash — `sha512-<base64>` SRI, exactly as npm/Bun record it — is written **verbatim** into `fetchurl { hash = "sha512-..."; }`. No re-hashing, no network call at generation time. Nix's `fetchurl` natively accepts SRI-format hashes including the `sha512-` prefix, so this is a direct pass-through of Bun's own trust data into Nix's trust data.
- **Git / GitHub / plain-tarball dependencies**: `bun.lock` records **no integrity hash at all** for these (only a URL/ref). `bun2nix` therefore shells out to `nix --extra-experimental-features 'nix-command flakes' flake prefetch <url> --json` at **`bun2nix` CLI run time** (`programs/bun2nix/src/lockfile/package_deserializer/prefetch.rs`, `Prefetch::prefetch_package`) to fetch the object, compute its hash, and record it in the generated `bun.nix`. This means:
  - Generating (not building) `bun.nix` for a lockfile with git/tarball deps **requires network access and a working `nix` binary** on the machine that runs `bun2nix`.
  - The resulting hash is genuinely *derived independently by Nix*, not "reused" from any upstream-supplied field — there is nothing to reuse, since Bun's lockfile doesn't carry one for these kinds.
  - A `warn!` is emitted every time this happens, pointing at [oven-sh/bun#19519](https://github.com/oven-sh/bun/issues/19519) (tracking upstream Bun exposing these hashes itself) and suggesting `RUST_LOG=off` to silence it.
  - The WASM/npm build of the CLI (`bunx bun2nix`) **cannot** do this at all — `Error::UnsupportedWASMCliAction`, because it can't spawn the `nix` subprocess — so the docs (`docs/src/using-the-command-line-tool.md`) explicitly tell users to switch to the native CLI once they have any git/tarball/non-default-registry dependency.
- **File / workspace-local packages**: no fetch, no hash — rendered as `copyPathToStore <prefix><path>` (a plain Nix store copy of a local path), using the `--copy-prefix`/`-c` CLI flag (default `./`) to make the path relative to wherever `bun.nix` will be evaluated from.

### Five fetcher kinds total (`Fetcher` enum, `fetcher.rs:17-68`)

`FetchUrl` (npm tarball, `pkgs.fetchurl`), `FetchGit` (`pkgs.fetchgit`), `FetchGitHub` (`pkgs.fetchFromGitHub`), `FetchTarball` (`builtins.fetchTarball`, used for plain HTTP(S) tarball deps), `CopyToStore` (`copyPathToStore`, for workspace/file deps). Each has its own `.nix_template` file and is a **fixed-output derivation** per package (or a plain store copy for local paths) — see §3.

One explicit design note from `docs/src/building-packages/fetchBunDeps.md`: npm packages deliberately use `pkgs.fetchurl` (fetch-only) rather than the more convenient `builtins.fetchTarball` (fetch+extract in one step), *because* `fetchTarball`'s hash covers the extracted tree, which would not match the tarball-level integrity hash recorded in `bun.lock`. Extraction is therefore a separate Nix build step (§3, step 2).

---

## 3. The actual Nix-side mechanism: rebuilding Bun's own cache, not running `bun install --frozen-lockfile` against the network

This is the load-bearing finding, mirroring this repo's `DEPS.md` §12. Read from `nix/fetch-bun-deps.nix`, `nix/fetch-bun-deps/{build-package,cache-entry-creator,extract-package}.nix`, `nix/mk-derivation/hook.{nix,sh}`, and `programs/cache-entry-creator/src/main.zig`.

`bun2nix` does **not** try to convince `bun install` to run "as normal" against a pre-warmed local npm registry or a rewritten `node_modules`. Instead it reconstructs Bun's own **global install cache** directory structure ([bun's cache docs](https://github.com/oven-sh/bun/blob/642d04b9f2296ae41d842acdf120382c765e632e/docs/install/cache.md)) entirely inside the Nix store, then points `bun install` at that cache and lets Bun believe everything it needs is already present:

1. **Fetch** (`fetchBunDeps` per-system flake-parts option, `nix/fetch-bun-deps.nix`): `pkgs.callPackage bunNix { fetchurl = <auth-wrapping fetchurl>; }` evaluates the generated `bun.nix`, producing one FOD (or store-copy) per package, exactly as described in §2.
2. **Extract + normalize** (`build-package.nix` → `extract-package.nix`): each fetched package (tarball or directory) is extracted with `bsdtar` (stripping the top path component) and `chmod -R u+rwx`'d, then optionally has shebangs patched (`patchShebangs`) and ELF binaries fixed up (`autoPatchelfHook`, opt-in via `autoPatchElf`).
3. **Re-key into Bun's cache-directory naming scheme** (`cache-entry-creator.nix` → the Zig binary `programs/cache-entry-creator/src/main.zig`): this is the crux. Bun's on-disk cache key for an npm package is **not** just `name@version` — for prerelease/build-metadata versions it's hashed with Bun's own vendored [Wyhash](https://github.com/oven-sh/bun/blob/755b41e85bec1744dc2f438f1dfd0e9152d7b62c/src/wyhash.zig) implementation and suffixed `@@@1` (or `@@<registry-host>@@@1` for non-default registries); git/GitHub/tarball deps get their own `@G@<rev>`, `@GH@<owner>-<repo>-<rev>`, `@T@<wyhash-of-url>@@@1` shapes. `bun2nix` **reimplements Bun's exact key-derivation algorithm in Zig** (`cachedFolderPrintBasename` and friends, with unit tests asserting bit-for-bit agreement with real Bun cache folder names) specifically because getting this wrong means Bun's installer won't recognize the cache entry and will try to hit the network. The tool is Zig (not Rust) expressly because it vendors/reimplements Bun's own Wyhash variant to match byte-for-byte.
4. **Symlink farm**: `cache-entry-creator` symlinks each normalized package into `$out/share/bun-cache/<bun-cache-key>`, and `symlinkJoin` combines all packages into one `bun-cache` derivation — the return value of `fetchBunDeps`.
5. **Force Bun to use it** (`nix/mk-derivation/hook.sh`, a Nix setup-hook installed via `bun2nix.hook`/`bun2nix.mkDerivation`): at build time, `bunSetInstallCacheDirPhase` copies (`cp -r`, to dereference symlinks) that cache into a fresh `mktemp -d`, exports it as `BUN_INSTALL_CACHE_DIR`, then `bunNodeModulesInstallPhase` runs a completely ordinary `bun install --linker=isolated [--backend=symlink on Darwin] --ignore-scripts` **inside the sandboxed derivation**. Because `BUN_INSTALL_CACHE_DIR` already contains every package Bun would otherwise fetch, keyed exactly as Bun expects, the installer never needs network access — it just links from the fake cache into `node_modules`.

So: **Bun's own offline/cache story is the offline mechanism, not a Nix-specific one.** `bun2nix` doesn't invent a new bun-install-offline mode; it exploits the fact that Bun already has one (`BUN_INSTALL_CACHE_DIR` + content-addressed cache folder names) and does the (nontrivial, Wyhash-dependent) work of pre-populating that cache from Nix-store-fetched, hash-verified content.

Note this is *not* `bun install --frozen-lockfile` in the sandboxed build itself — that flag appears only in template `devShells` (`shellHook = "bun install --frozen-lockfile";`, e.g. `templates/catalog/flake.nix`) for interactive/dev use outside the sandbox. Inside the actual package derivation, the hook runs plain `bun install --linker=isolated --ignore-scripts`, relying on the pre-populated cache rather than a frozen-lockfile network-refusal flag to guarantee no drift.

`bun2nix.mkDerivation` (`nix/mk-derivation.nix`) layers a `bun build --compile` step on top of the hook for producing standalone Bun-compiled binaries; `bun2nix.hook` alone is recommended for anything that isn't a single-binary compile target (e.g. a bundled website).

---

## 4. Why this tool exists: the gap between Bun and Nix

Stated implicitly throughout, and explicitly in `docs/src/building-packages/fetchBunDeps.md` and the hook's own error message (`nix/mk-derivation/hook.sh:6-30`):

- `bun install` (like `npm install`/`yarn install`) wants to talk to an npm-compatible registry over the network to resolve and download tarballs.
- Nix derivation builds are sandboxed with no network access (outside of hash-verified fixed-output derivations), by design, for reproducibility.
- Without `bun2nix`, a Nix build of a Bun project has no dependencies available and `bun install` simply fails (or worse, is allowed network access via `--impure`-style escape hatches, defeating reproducibility).
- `bun2nix`'s answer: shift *all* network fetching into per-package Nix fixed-output derivations (hash-verified, reproducible, cacheable, substitutable) ahead of time, then make the actual `bun install` step believe it's a warm-cache no-op by faithfully reconstructing Bun's own cache layout (§3). The lockfile (`bun.lock`) is the single source of truth for exactly which package/version/hash to fetch, matching the "reproducibility guarantee" role that this repo's `DEPS.md` §7 describes for Gossamer's own `project.lock` + `--locked`.

---

## Input / Output summary

**Input** (ecosystem-native, produced entirely by Bun, never touched/regenerated by `bun2nix`):
- `bun.lock` — Bun ≥ 1.2's JSON/JSONC textual lockfile. Fields consumed: `lockfileVersion`, `workspaces.*.{name,dependencies,devDependencies}`, and every `packages.<id>` tuple (arity-dispatched: workspace / file-or-tarball / git-or-github-or-tarball / npm-registry-with-integrity). For npm packages specifically: the `sha512-` SRI integrity hash is read and trusted as-is.
- (Never consumed: `bun.lockb`, the older binary lockfile format — out of scope entirely, not merely deprioritized.)

**Output** (Nix-consumable):
- `bun.nix` — a Nix function `{ copyPathToStore, fetchFromGitHub, fetchgit, fetchurl, ... }: { "<pkg-id>" = <fetcher-call>; ... }`, one fixed-output-derivation-producing fetcher call (`fetchurl`/`fetchgit`/`fetchFromGitHub`/`builtins.fetchTarball`) or plain store copy (`copyPathToStore`) per lockfile package.
- At `fetchBunDeps` evaluation time, that turns into a single `bun-cache` derivation: a symlink farm laid out exactly like Bun's real `~/.bun/install/cache`, using Bun's own (reimplemented-in-Zig) Wyhash-based cache-key naming.
- Consumed at build time by a Nix setup hook that runs an ordinary, unmodified `bun install` inside the sandbox against that pre-populated cache — no code path re-implements or bypasses Bun's installer; it is fed a cache Bun mistakes for its own.

**The single most important input→output insight**: `bun2nix` does not treat "install offline" as a problem to solve *instead of* `bun install` — it solves it *for* `bun install`, by faithfully reverse-engineering Bun's own on-disk cache format (including a byte-exact reimplementation of Bun's internal Wyhash-based cache-key algorithm in a dedicated Zig helper) and populating that cache from Nix fixed-output derivations. Hash provenance follows the same split as source-kind provenance: npm packages get their Nix hash for free by copying Bun's own `sha512` SRI integrity field verbatim, because Bun/npm already record a strong, checkable hash; everything else (git, GitHub, raw tarball URLs) has no such field in `bun.lock`; and for those, `bun2nix` must call out to `nix flake prefetch` at generation time to mint a hash. That's a real, unavoidable network/tooling dependency of running the CLI itself (not the sandboxed build) whenever non-npm dependency kinds are present — a genuine gap between what Bun's lockfile records and what a Nix fixed-output derivation requires (a direct analogue of this repo's `DEPS.md` §12 finding about Gossamer's own lockfile/build-time gap).

---

## 5. Limitations / gotchas actually documented or found in source

- **`catalog:` dependency specifiers** (Bun's shared-version-catalog feature): not handled by the Rust lockfile parser at all. Handled entirely by a helper TypeScript script (`nix/mk-derivation/resolve-catalog.ts`) invoked from the setup hook (`bunResolveCatalogRefs` in `hook.sh`) *before* `bun install` runs, because — per the hook script's own comment — "bun re-resolves `catalog:` dependency specifiers against the npm registry on every `bun install`, even with a fully populated cache," which fails inside the sandbox; the workaround rewrites every `catalog:` reference in `bun.lock`'s `workspaces` section and every workspace `package.json` to the exact resolved version before install.
- **`patchedDependencies`** (Bun's patch-package-equivalent feature): also stripped from `package.json`/`bun.lock` by the hook before `bun install` (`hook.sh:108-116`, via `yq`), because patches are applied *ahead of time* to the Nix-store package derivation instead (via `fetchBunDeps`'s `overrides` mechanism plus `patchedDependenciesToOverrides`, `nix/fetch-bun-deps/{override-package,patched-dependencies-to-overrides}.nix`) — patch hash suffixes in bun's normal cache-key scheme are deliberately avoided so cache lookups hit the un-suffixed key.
- **Optional/platform-specific deps**: not specifically called out in the source read for this document; not confirmed either way — flag as **unconfirmed**.
- **Non-default/private registries**: supported via an explicit `tarballUrl` field per package (arity-4 npm entries) plus `bunfig.toml`/`.npmrc` credential parsing on the Nix side (`nix/mk-derivation/hook.nix`'s `parseBunfigCredentials`/`parseNpmrcCredentials`, wrapping `fetchurl` with an `Authorization: Bearer` header) — see `templates/private-registry/`.
- **Binary lockfile (`bun.lockb`)**: categorically unsupported (§1) — not a partial-support gap, a hard scope boundary. Bun itself defaults to the text `bun.lock` format from v1.2 onward, which is presumably why this was an acceptable scope cut for the tool's authors.
- **WASM/npm-distributed CLI** (`bunx bun2nix`) cannot process any lockfile entry that requires `nix flake prefetch` (git, GitHub, plain-tarball, or non-default-registry deps needing a fresh hash) — `Error::UnsupportedWASMCliAction` — users are told to fall back to the native CLI for such projects (§2, §1 disambiguation table in `using-the-command-line-tool.md`).
- **Manifest/schema stability**: `bun.nix`'s schema is explicitly *not* guaranteed stable across `bun2nix` versions (the hook's own error text for eval failures tells users to just regenerate it) — unlike, say, a lockfile meant for long-term diffing, `bun.nix` is treated as a disposable build artifact.

---

## Sources

- [nix-community/bun2nix](https://github.com/nix-community/bun2nix), `master` branch, commit `0f2a1f0b6f42cebe3b149bf62d38754c5e0e9729` (2026-07-21), tag `2.1.2` — cloned locally and read directly, not summarized secondhand.
  - `programs/bun2nix/src/{lockfile,lockfile/package_deserializer,lockfile/package_deserializer/prefetch,lockfile/package_visitor,package,package/fetcher,nix_expression,main,error}.rs`
  - `programs/bun2nix/templates/*.nix_template`
  - `programs/cache-entry-creator/src/main.zig`
  - `nix/{fetch-bun-deps,fetch-bun-deps/*,mk-derivation,mk-derivation/hook.nix,mk-derivation/hook.sh}.nix`
  - `docs/src/{bun2nix,installation,using-the-command-line-tool,v2-update-guide,building-packages,building-packages/{fetchBunDeps,mkDerivation,hook}}.md`
  - `templates/{default,git-deps,workspace,catalog,patched-deps,patched-deps-scoped,private-registry,tarball-deps}/{bun.lock,bun.nix,flake.nix}`
  - `flake.nix`, `README.md`
- `https://github.com/baileyluTCD/bun2nix` → confirmed HTTP 301 redirect to `https://github.com/nix-community/bun2nix` via `curl -sI` (2026-07-26).
- [Bun's global cache docs](https://github.com/oven-sh/bun/blob/642d04b9f2296ae41d842acdf120382c765e632e/docs/install/cache.md) and [oven-sh/bun#11863](https://github.com/oven-sh/bun/issues/11863) (text lockfile), [oven-sh/bun#19519](https://github.com/oven-sh/bun/issues/19519) (tracking upstream Bun exposing hashes for non-npm deps) — linked in-source by `bun2nix` itself, not independently verified against Bun's own repo for this document.
- WebSearch results surfacing `aabccd021/bun2nix-rs`, `aabccd021/bun3nix`, `nyadiia/bun2nix`, and the [NixOS Discourse Bun3nix announcement](https://discourse.nixos.org/t/bun3nix-bun-lock-to-nix-with-tailwindcss-plugins-support/69939) — used only for the disambiguation in §0; their internals were not read for this document.

Sections flagged **unconfirmed** above (optional/platform-specific dependency handling) should be re-checked against current upstream source before being relied on.
