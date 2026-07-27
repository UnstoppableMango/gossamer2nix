# Dependency Management in `poetry2nix`

Read 2026-07-26 (`master` @ `ce2369d`, 2025-04-03, confirmed current tip) as prior art and structural contrast for `gossamer2nix`. Sources: `default.nix`, `lib.nix`, `mk-poetry-dep.nix`, `editable.nix`, `fetchers/default.nix`, `hooks/default.nix`, `overrides/default.nix` (~4,500 lines), the vendored `pyproject.nix` platform-matching code, `docs/edgecases.md`, and issue #1865 (the maintainer's own retrospective).

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/poetry2nix](https://github.com/nix-community/poetry2nix) (`master`) |
| **Ecosystem** | Python / Poetry |
| **Maintenance status** | Unmaintained — original maintainer stepped down Nov 2024 ([issue #1865](https://github.com/nix-community/poetry2nix/issues/1865)), no commits since 2025-04-03; recommends `uv` + `uv2nix` instead |
| **Ecosystem input** | `pyproject.toml` (metadata/markers only) + `poetry.lock` (`[[package]]` name/version/`files`-with-hashes/`source`/`dependencies`) |
| **Generated output** | No single file — an **overlay** over nixpkgs' `python.pkgs` set via `python.override { packageOverrides }` |
| **Hash strategy** | Reused verbatim — `poetry.lock`'s `sha256:<hex>` per-file hash matches Nix's own `outputHash` grammar exactly (`algo:hex`), a syntactic no-op |
| **Nix build mechanism** | One `buildPythonPackage` derivation per locked package, composed with ~4,500 lines of hand-written per-package overrides for build-backend diversity |

**Does no dependency resolution at all** — confirmed by reading the source: there is no resolver here. All resolution happened earlier, inside `poetry lock` (a Python/Rust tool entirely outside Nix); `poetry2nix` never re-derives or re-validates that decision, it just replays it. This is the single biggest structural difference from Gossamer's own resolver (`../DEPS.md` §6).

## Input

`pyproject.toml` (`builtins.fromTOML`) supplies project metadata, PEP 508 marker/version-constraint text (used only for filtering, never to pick versions — that's fully delegated to the lock), extras-to-dependency mapping, and `build-system.requires` (the project's own PEP 517 backend, resolved to a Nix package). `poetry.lock` supplies the actual resolved graph: `name`, `version`, `files` (inline per-file `hash = "sha256:..."` in lock format 2.0+, or a separate `[metadata.files]` table in the older 1.1 format), `source` (nullable — git/url/directory/file/legacy), transitive `dependencies` with their own markers.

**Confirmed absence**: `[metadata.content-hash]` — Poetry's own field for detecting whether `poetry.lock` is stale relative to `pyproject.toml` — is never read or verified anywhere in the Nix code (`grep` across every `.nix` file, including the vendored parsing library, matches nothing outside lockfile fixture data). This is poetry2nix's own version of the gap `../DEPS.md` §12 documents for `gos build`: the lockfile carries a staleness field the Nix consumer silently ignores, with no equivalent of Gossamer's `--locked` drift check.

Parsing itself is delegated to a separate, actively-maintained library by the same original author, [`pyproject-nix/pyproject.nix`](https://pyproject-nix.github.io/pyproject.nix/) (vendored, not a submodule): PEP 508 marker evaluation, PEP 440 version comparison, PEP 503 name normalization, and wheel/platform-tag parsing all live there rather than being reimplemented in `poetry2nix` itself.

## Output and hash mapping

Per-package fetching is dispatched by `source.type`: absent → PyPI (`fetchFromPypi`, a custom FOD), `"git"` → `builtins.fetchGit` pinned to the resolved commit, `"url"` → `builtins.fetchurl`, `"directory"`/`"file"` → direct filesystem path, no fetch. For the common PyPI case:

```nix
outputHashMode = "flat";
outputHashAlgo = "sha256";
outputHash = hash;   # taken verbatim from poetry.lock's `hash = "sha256:<hex>"` field
```

No conversion code exists because none is needed — Poetry's `"sha256:<hex>"` is already the exact string Nix's `outputHash` grammar accepts. This is a coincidence of this particular ecosystem's lock format, not a generalizable trick — Gossamer's own `sha256` (`../DEPS.md` §7) is a bare hex digest with no algorithm prefix, so `gossamer2nix` will need an actual conversion step poetry2nix gets for free. Only one file per package (whichever wheel matches the running platform, or the sdist) is ever fetched or hash-checked; the lockfile's other recorded hashes for unselected platforms/formats are simply unused.

## Nix-side mechanism: overlay, and where the real complexity lives

`mkPoetryPackages` builds an overlay chain over `python.pkgs`: null out colliding nixpkgs attributes, inject the per-package builder, force `doCheck = false` (dodges check-dependency recursion), null out packages excluded by marker/version evaluation for the target interpreter, build one derivation per compatible locked package, then layer user overrides (default: `defaultPoetryOverrides`) on top. Every derivation is `buildPythonPackage`, with `format` set per-package: `"wheel"` skips compilation entirely; source builds pull in custom setup hooks that strip non-registry dependency specs `pip` inside the sandbox can't resolve, then run `pip wheel --no-index --no-deps --no-build-isolation` against whatever PEP 517 backend the package declares.

**This build-backend diversity, not fetching, is where the overwhelming majority of the codebase's complexity lives.** PyPI metadata routinely omits real build-time requirements (a package needing `setuptools` but never declaring it is called out in the docs as the canonical failure), so `overrides/default.nix` (~4,500 lines) hand-fixes packages one at a time — missing `nativeBuildInputs`, system libraries no Python metadata could ever express (BLAS/LAPACK for `numpy`, `libpq` for `psycopg2`), even a separately-maintained Cargo-hash lookup table for Rust-extension packages built via `maturin`/`setuptools-rust`. Wheel platform selection is a second, orthogonal axis: a wheel's filename platform tag (`manylinux*`, `musllinux*`, `macosx_*`, ...) is matched against the target platform at Nix-evaluation time, picking exactly one file per package for that specific `python`/platform combination — no cross-building of multiple variants within one evaluation.

## Why the tool exists

`pip`/`poetry install` want full network access to PyPI at install time; Nix sandboxes builds. poetry2nix moves every fetch into individually hash-pinned FODs ahead of time, then feeds only already-fetched sources to a `pip wheel --no-index` build — the same fetch-then-verify-then-build shape `../DEPS.md` §8-9 describes for Gossamer. But because dependency resolution is fully out of scope (trusted verbatim from `poetry.lock`), the entire remaining problem reduces to "how do I turn an already-resolved, already-hashed graph into buildable, sandboxed derivations" — which is dominated by PEP 517 backend diversity, not hashing.

## Limitations, and the maintainer's own retrospective

- **Overrides silently supersede the lockfile**: the docs' own worked example shows an override changing a package's version builds successfully even though it no longer matches what `poetry.lock` says was resolved — no drift check exists.
- **`extras`-with-shared-dependency infinite recursion**: a documented real case (`dask[distributed]`) requiring a manual workaround.
- Rust/Cargo extension packages need a second, hand-maintained hash table with "still needs human-in-a-loop" acknowledged directly in the docs; the escape hatch (`preferWheels = true`) explicitly trades reproducible source builds for prebuilt-wheel trust.
- The original maintainer's own issue #1865 is unusually candid: dependency solving in Poetry itself "isn't very good," the nixpkgs Python builders "aren't good" and should be replaced, the `mkPoetry*` API is "misdesigned" (a Python-to-Nix tool "shouldn't" ship an opinionated default like sdist-vs-wheel), and the ~4,500-line override table is "my biggest annoyance in regards to maintenance" — the retrospective's position is that a `*2nix`-style tool shouldn't ship overrides papering over lockfile deficiencies at all; that should live in external, ephemeral projects instead.

## Relevance to `gossamer2nix`

poetry2nix cleanly separates "replay an already-resolved lock as pinned Nix fetches" (mechanical, ~fully solved, and the part that most resembles what `gossamer2nix` needs for Gossamer's registry/git/tarball deps) from "actually build each dependency inside a sandbox" (the part that consumed the project's entire complexity budget and, per the maintainer, was never solved well). Gossamer's simpler single-toolchain build model suggests `gossamer2nix` may avoid poetry2nix's biggest cost center — but only if `../DEPS.md` §12's open question resolves in favor of non-path dependencies actually being wired into `gos build`, and only if Gossamer's own `[rust-bindings]` FFI path doesn't reproduce a small slice of this same problem.
