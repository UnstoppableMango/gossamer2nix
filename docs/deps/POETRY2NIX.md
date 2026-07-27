# Dependency Management in `poetry2nix`

This document was compiled by reading the source of [nix-community/poetry2nix](https://github.com/nix-community/poetry2nix) directly (shallow `git clone` of `master`, commit `ce2369d`, dated 2025-04-03 — confirmed via `gh api repos/nix-community/poetry2nix/commits` on 2026-07-26 to be the current tip of `master`; no commits since then), plus `gh api` reads of the repo's maintenance issue and repo metadata.
It exists to compare `poetry2nix`'s design against Gossamer's own dependency model (see [`DEPS.md`](../DEPS.md)) as prior art for this repo's `gossamer2nix` deps-lock/Nix adapter (see [`GOALS.md`](../../GOALS.md)).

**Canonical repo**: `nix-community/poetry2nix` (`github.com/nix-community/poetry2nix`, default branch `master`, not archived, not a fork — `gh api repos/nix-community/poetry2nix --jq '{archived,fork,default_branch}'` → `{"archived":false,"fork":false,"default_branch":"master"}`).
There is at least one unrelated fork (`l0b0/poetry2nix`) that surfaced in search results; it is not the project referenced here.

**Maintenance status: unmaintained**, per the repo's own README and a still-open maintainer issue:

> ## Maintenance status: ⚠️ Unmaintained ⚠️
> The creator & long-term maintainer of poetry2nix ([@adisbladis](https://github.com/adisbladis)) is no longer using Poetry or Poetry2nix.
> This means that [poetry2nix is looking for maintainers](https://github.com/nix-community/poetry2nix/issues/1865).
> — [`README.md`](https://github.com/nix-community/poetry2nix/blob/master/README.md), lines 10–14

Issue [#1865](https://github.com/nix-community/poetry2nix/issues/1865) (opened 2024-11-07, still open as of this reading) is the maintainer's own retrospective and is unusually candid, self-critical prior art — quoted at length in §9 below because several of its complaints bear directly on `gossamer2nix`'s design choices.
The maintainer's stated recommendation for new projects is to use [`uv`](https://docs.astral.sh/uv/) + [`uv2nix`](https://github.com/pyproject-nix/uv2nix) instead — itself worth a future survey doc in this series.

**Confirmed via direct source reading — no code path was found that reads the resolution algorithm itself, because there isn't one**: `poetry2nix` does not resolve dependencies at all.
This is the single biggest structural difference from `gossamer2nix`'s planned scope and from Gossamer's own resolver (`DEPS.md` §6): `poetry2nix` is a pure lockfile-to-derivation compiler.
All resolution happened earlier, inside `poetry lock`, a Python/Rust tool outside Nix entirely; `poetry2nix` never re-derives or re-validates that decision, it just replays it.

---

## 1. What `poetry2nix` is

> _poetry2nix_ turns [Poetry](https://python-poetry.org/) projects into Nix derivations without the need to actually write Nix expressions. It does so by parsing `pyproject.toml` and `poetry.lock` and converting them to Nix derivations on the fly.
> — [`README.md`](https://github.com/nix-community/poetry2nix/blob/master/README.md), line 8

It ships as a Nix flake (`flake.nix`) exposing `lib.mkPoetry2Nix { pkgs }` (a `lib.makeScope`-based attribute set, `default.nix`) and an overlay (`overlays.default`, `overlay.nix`), plus a small Python CLI (`bin/poetry2nix`, wired up via `cli.nix`) used only to help supplement SHA-256 hashes for git-sourced dependencies (see §5).

Primary entry points (all in `default.nix`):

| Function | Purpose |
|---|---|
| `mkPoetryPackages` | Core: returns `{ python, poetryPackages, pyProject, poetryLock }` — an overridden Python interpreter whose package set (`python.pkgs`) is populated from `poetry.lock`, plus the flat list of resolved packages |
| `mkPoetryApplication` | Wraps `mkPoetryPackages` in `buildPythonPackage` to build the project itself as an installable app/package |
| `mkPoetryEnv` | Wraps `mkPoetryPackages` in `python.withPackages` to produce a dev-shell-style Python environment |
| `mkPoetryEditablePackage` | Produces a `.pth`/`.egg-info`-based editable install shim for local development |
| `mkPoetryScriptsPackage` | Materializes `[tool.poetry.scripts]` entry-point wrapper scripts |
| `defaultPoetryOverrides` | ~4,500 lines (`overrides/default.nix`) of hand-maintained per-package Nix build fixes, see §6 |

---

## 2. Input

`poetry2nix` reads exactly two files per project, both produced by upstream Poetry, never by `poetry2nix` itself:

### 2.1 `pyproject.toml`

Read via `readTOML` (`lib.nix`), which is `builtins.fromTOML`, falling back to a `remarshal`-based Nix derivation shim on Nix versions without native TOML support.
Fields consumed (from `mkInputAttrs` and `mkPoetryPackages` in `default.nix`):

- `tool.poetry.name`, `.version`, `.description`, `.homepage`, `.license` — package/app metadata
- `tool.poetry.dependencies` (and legacy `tool.poetry.dev-dependencies`, plus Poetry ≥1.2 `tool.poetry.group.<name>.dependencies`) — used only to recover **PEP 508 marker context and version constraints for filtering**, e.g. `sourceSpec` lookups and `checkGroups`; not to decide *which* versions to pull (that's fully delegated to the lockfile, see below)
- `tool.poetry.extras` — extras-to-dependency-name mapping, consulted when the caller passes `extras` (default `[ "*" ]`, meaning all)
- `tool.poetry.scripts`, `tool.poetry.plugins` — for `mkPoetryScriptsPackage` / editable installs
- `build-system.requires` — read by `getBuildSystemPkgs` (`lib.nix`) to resolve the project's own PEP 517 build backend (e.g. `poetry-core`) to a Nix package attribute, with a hard `throw` if the section is missing

### 2.2 `poetry.lock`

Also read via `readTOML`. `poetry2nix` supports both lockfile schema generations (`mkPoetryPackages`, `default.nix` lines 166–226):

- **Lock format 2.0+** (current): each `[[package]]` entry carries its own `files = [ { file = "...", hash = "sha256:..." }, ... ]` array inline.
- **Lock format 1.1** (older Poetry): per-file hashes instead live in a separate top-level `[metadata.files]` table keyed by (unnormalized) package name; `poetry2nix` falls back to it with `pkgMeta.files or lockFiles.${normalizedName}`.

Per-`[[package]]` fields actually consumed (`mk-poetry-dep.nix` function signature, lines 12–26): `name`, `version`, `files` (or the 1.1 fallback), `source` (nullable; `{ type, url, reference, resolved_reference, subdirectory }` for git/url/directory/file/legacy sources), `dependencies` (transitive dep table with per-dep `python`/`markers` constraints), `python-versions`, `extras`.

Example (lock format 2.0, from `tests/trivial/poetry.lock`):

```toml
[[package]]
name = "click"
version = "7.1.2"
description = "Composable command line interface toolkit"
optional = false
python-versions = ">=2.7, !=3.0.*, !=3.1.*, !=3.2.*, !=3.3.*, !=3.4.*"
files = [
    {file = "click-7.1.2-py2.py3-none-any.whl", hash = "sha256:dacca89f4bfadd5de3d7489b7c8a566eee0d3676333fbb50030263894c38c0dc"},
    {file = "click-7.1.2.tar.gz", hash = "sha256:d2b5255c7c6349bc1bd1e59e08cd12acbbd63ce649f2588755783aa94dfb6b1a"},
]

[metadata]
lock-version = "2.0"
python-versions = "^3.9"
content-hash = "a97f8c9028114be71b45d3362871c4bcc471b4e9a4ded2508e7f3a80f4060766"
```

**Confirmed absence, checked directly**: `grep -rl "content-hash" .` across every `.nix` file in the repo (including the vendored `pyproject.nix` library, §3) matches nothing outside `poetry.lock` fixture data itself.
`[metadata.content-hash]` — the hash Poetry itself uses to detect whether `poetry.lock` is stale relative to `pyproject.toml` — is **never read or verified by any Nix code path in `poetry2nix`**.
This is `poetry2nix`'s own version of the gap `DEPS.md` §12 documents for `gos build`: the lockfile carries a staleness-detection field, but the Nix consumer silently ignores it and will happily build against a `poetry.lock` that no longer matches `pyproject.toml`, with no equivalent of Gossamer's `--locked` drift check.

---

## 3. Architecture note: `pyproject.nix` as the parsing engine

`poetry2nix` vendors a subset of a separate, actively-maintained project by the same original author, [`pyproject-nix/pyproject.nix`](https://pyproject-nix.github.io/pyproject.nix/), at `vendor/pyproject.nix/` (plain vendored files — `lib/` + `default.nix` — not a git submodule).
All PEP-standard parsing/matching logic is delegated to it rather than reimplemented:

- `pyproject-nix.lib.pep508` — marker parsing/evaluation (`parseMarkers`, `evalMarkers`, `mkEnviron`) and requirement-string parsing (`parseString`)
- `pyproject-nix.lib.pep440` — version parsing and comparator functions, used both for the interpreter's own version and for evaluating Poetry's caret/tilde-style version conditions (`pyproject-nix.lib.poetry.parseVersionCond`)
- `pyproject-nix.lib.pypa` — package-name normalization (PEP 503, `normalizePackageName`), wheel/egg filename parsing, and **platform-tag-to-Nix-platform matching** (`isPlatformTagCompatible`, `selectWheels`) — see §7
- `pyproject-nix.lib.eggs` — legacy `.egg` filename parsing

This split is itself informative for `gossamer2nix`: `poetry2nix` factors "understand this ecosystem's file formats and version grammar" into a reusable, ecosystem-general library separate from "turn a lockfile into derivations," and the maintainer's own retrospective (§9) says the Nix-build-mechanics half (`buildPythonPackage`/`overridePythonAttrs`) should have been factored out the same way but wasn't — that's now [`pyproject.nix`'s `build.html`](https://pyproject-nix.github.io/pyproject.nix/build.html) approach in the intended successor design.

---

## 4. Output

`poetry2nix` produces **one Nix derivation per locked package**, assembled into a Python interpreter's package set via [`python.override { packageOverrides }`](https://nixos.org/manual/nixpkgs/stable/#python) — i.e. it is structurally an **overlay over nixpkgs' Python package set**, not a from-scratch package set (`default.nix` lines 199–278, `baseOverlay`).

The overlay-composition order (`overlays = builtins.map getFunctorFn [ ... ]`, `default.nix` lines 233–276) is, in sequence:

1. Null out any pre-existing nixpkgs attribute whose *normalized* name differs from its raw name (avoids collisions between nixpkgs' and `poetry2nix`'s naming).
2. Inject `mkPoetryDep` (the per-package builder, §5–7) and `fetchFromPypi` (§5) into the package set as callable attributes.
3. Force `doCheck = false` on every derivation with `overridePythonAttrs` (works around infinite recursion from check-dependency cycles).
4. Null out packages whose PEP 508 markers / Python-version constraints exclude them for the target interpreter (`incompatible` from `lib.partition`, §7).
5. `baseOverlay` — build one `mkPoetryDep` derivation per **compatible** `[[package]]` entry from `poetry.lock` (§7 covers the compatibility filter), keyed by normalized name.
6. User-supplied `overrides` (defaults to `defaultPoetryOverrides`, §6), applied last so they win.

`mkPoetryPackages` returns the resulting `python` interpreter plus `poetryPackages` — `storePackages`, computed via nixpkgs' `requiredPythonModules` over the direct-dependency subset selected by `groups`/`extras`, which walks and flattens the *transitive* `propagatedBuildInputs` graph that steps 5–6 built. `mkPoetryApplication`/`mkPoetryEnv` are thin wrappers turning that package set into, respectively, a `buildPythonPackage` derivation for the project itself or a `python.withPackages` environment.

---

## 5. Fetching and hash mapping

Per-package source fetching is dispatched in `mk-poetry-dep.nix` (`src = ...`, lines 191–245) by `poetry.lock`'s `source.type` (or its absence, meaning "PyPI"):

| `source.type` | Nix fetch mechanism |
|---|---|
| *(none — PyPI)* | `fetchFromPypi` (custom fixed-output derivation, `fetchers/default.nix`) or `fetchPypiLegacy` (Python-script-driven, for non-standard/legacy indices) |
| `"git"` | `builtins.fetchGit`, pinned to `source.resolved_reference or source.reference` |
| `"url"` (including wheel URLs) | `builtins.fetchurl` |
| `"directory"` | direct filesystem path (`pwd + source.url`), passed through `cleanPythonSources` (gitignore/pycache-aware filtering, `lib.nix`) |
| `"file"` | direct filesystem path, no fetch |
| `"legacy"` | `fetchPypiLegacy` with the file/hash/url triple |

### 5.1 `fetchFromPypi` and the hash format

`fetchers/default.nix` predicts the PyPI file URL deterministically (`https://files.pythonhosted.org/packages/<kind>/<first-letter>/<pname>/<file>`, where `kind` is `wheel`/`sdist`/egg-implementation derived from the filename) and fetches it as a **fixed-output derivation**:

```nix
outputHashMode = "flat";
outputHashAlgo = "sha256";
outputHash = hash;   # taken verbatim from poetry.lock's `hash = "sha256:<hex>"` field
```

**No explicit hash-format conversion code exists, because none is needed**: Poetry's own lockfile hash encoding, `"sha256:<hex>"`, is already the exact string form Nix's own hash grammar accepts for `outputHash` (Nix parses `algo:hex`, `algo-base64`/SRI, and bare-hex forms interchangeably when `outputHashAlgo` is also given). The mapping from ecosystem hash to Nix hash is a syntactic no-op here, not a computed translation — worth noting because it means this particular ecosystem's lock format happens to be Nix-native by coincidence, which will very likely **not** be true for Gossamer's lockfile (`DEPS.md` §7 records `sha256` as a bare hex digest with no algorithm prefix, so `gossamer2nix` will need an actual `algo:hex`/SRI conversion step that `poetry2nix` gets for free).

If the predicted URL 404s, `fetch-from-pypi.sh` falls back to querying PyPI's JSON API for the package to find the real URL, still verified against the same declared `outputHash`.

### 5.2 What is *not* verified

`selectWheel`/`fileInfo` (`mk-poetry-dep.nix` lines 84–110) pick **one** file entry per package from the lock's `files` array — whichever wheel matches the running platform (§7) or, absent a matching wheel, the sdist — and only that one entry's hash is ever fetched or checked. The other hashes recorded in the lockfile for platforms/formats not selected on this build are simply unused data as far as this build is concerned; there's no cross-platform verification pass.

---

## 6. Build-system diversity: `buildPythonPackage`, hooks, and 4,500 lines of overrides

This is the part of the Poetry→Nix gap with no equivalent complexity in Gossamer's model (`DEPS.md` never has to reconcile *N* different build backends), so it's worth documenting in some depth.

### 6.1 The base mechanism

Every package derivation is nixpkgs' `buildPythonPackage` (`mk-poetry-dep.nix` line 116), with `format` set per-package based on what was found in the lockfile:

- `"wheel"` — pre-built; skips compilation, uses `wheelUnpackHook` + `pypaInstallHook`, and disables stripping (`dontStrip`, since stripping pre-built wheels can corrupt ELF load commands).
- `"poetry2nix"` (an alias substituted in for `"pyproject"`, to "circumvent output separation" per a linked nixpkgs PR, line 120) — sdist/source builds. Pulls in four custom setup hooks (`hooks/default.nix`): `removePathDependenciesHook`, `removeGitDependenciesHook`, `removeWheelUrlDependenciesHook` (each strips non-registry dependency specs from `pyproject.toml` before the build, since pip inside the sandbox can't resolve or fetch them — see §8) and `pipBuildHook`, which runs `pip wheel --no-index --no-deps --no-build-isolation` (`hooks/pip-build-hook.sh`) to invoke whatever PEP 517 backend the package declares, entirely offline against packages already materialized as Nix store paths.
- `"egg"` — legacy egg format, detected by filename suffix.

The project's *own* build backend (from `build-system.requires` in its `pyproject.toml`) is resolved the same way, via `getBuildSystemPkgs` (§2.1), and added as a `buildInputs`/`nativeBuildInputs` entry — this is `poetry2nix`'s analogue of Gossamer's `[rust-bindings]` FFI-crate handling in spirit (a second, backend-specific input set alongside the main dependency graph) but here it's mandatory for every single package, not an opt-in FFI path.

### 6.2 Overrides: `overrides/default.nix`, ~4,521 lines

Because PEP 517 backend metadata on PyPI is frequently incomplete or wrong (a package needs `setuptools`/`hatchling`/`cython`/Cargo toolchains it never declares, or needs a system library that no Python metadata could ever express), `poetry2nix` ships a large table of **hand-written per-package Nix fixes**, composed as an overlay layered after the lockfile-driven base set (§4 step 6). Representative, directly-read examples:

```nix
# overrides/default.nix:2340 — needs libpq headers + openssl on Darwin
psycopg2 = prev.psycopg2.overridePythonAttrs (old: {
  buildInputs = old.buildInputs or [ ] ++ lib.optionals stdenv.isDarwin [ pkgs.openssl ];
  nativeBuildInputs = old.nativeBuildInputs or [ ] ++ [ pkgs.postgresql ];
});
```

```nix
# overrides/default.nix:1814 — numpy needs a Fortran compiler and a BLAS implementation
# wired via a generated site.cfg, neither expressible from pyproject.toml/poetry.lock alone
numpy = prev.numpy.overridePythonAttrs (old: let
  blas = old.passthru.args.blas or pkgs.openblasCompat;
  ...
in {
  nativeBuildInputs = old.nativeBuildInputs or [ ] ++ [ gfortran ];
  buildInputs = old.buildInputs or [ ] ++ [ blas ];
  ...
});
```

A separate `overrides/build-systems.json` (referenced from `docs/edgecases.md`) maps package name → list of missing PEP 517 build-time dependencies for the simpler, mechanical cases (e.g. "package X needs `setuptools-scm` to build, but doesn't declare it"), applied generically by `addBuildSystem'` in `hooks/default.nix`.

### 6.3 Rust extensions specifically

`docs/edgecases.md` documents a distinct sub-problem: Python packages with Rust extensions built via `setuptools-rust`/`maturin` need a **Cargo-level** vendored-dependency hash (`cargoHash`/`cargoSha256`-equivalent) in addition to the Python-level PyPI file hash, maintained via a `getCargoHash` lookup table that must be updated by hand per Rust-dependency-version. The doc's own advice when a hash is missing: pin the Python package to an older version in `pyproject.toml`, or fall back to `preferWheels = true` (skip building from source entirely, trust the prebuilt wheel's hash instead — explicitly flagged in the docs as a supply-chain trust trade-off).

---

## 7. Platform-specific wheels inside a cross-platform derivation model

Wheel selection is where PyPI's per-platform binary distribution model meets Nix's single-target-per-derivation-evaluation model. Handled in two layers:

1. **PEP 508 marker / Python-version compatibility** (`mkPoetryPackages`, `default.nix` lines 179–193): every `[[package]]` in the lock is partitioned into `compatible`/`incompatible` by evaluating its `marker` field (if present, via `pyproject-nix.lib.pep508.evalMarkers` against `pep508Env = pyproject-nix.lib.pep508.mkEnviron python`) and its `python-versions` field (via the vendored `checkPythonVersions`, `lib.nix`). Incompatible packages are nulled out of the package set entirely (§4 step 4) — this happens once, at Nix-evaluation time, for whichever single Python interpreter/platform `python` was passed in.

2. **Wheel platform-tag matching**, done by the vendored `pyproject-nix.lib.pypa` (`vendor/pyproject.nix/lib/pypa.nix`, `isPlatformTagCompatible`, confirmed by direct read, lines ~245–301): dispatches on the wheel filename's platform tag prefix — `manylinux*` (delegates to a `pep600.manyLinuxTagCompatible` check), `musllinux*` (`pep656.muslLinuxTagCompatible`), `macosx_*`, `win32`/`win_*`, `linux_*` — against `pkgs.python3.stdenv.targetPlatform` and the interpreter's libc. `selectWheels` (same file) filters+ranks candidate wheel files from `poetry.lock`'s `files` array down to the ones compatible with the Nix build's actual target platform; `selectWheel` in `mk-poetry-dep.nix` (lines 31–39) takes the first match. `getManyLinuxDeps` (`lib.nix` lines 30–38) separately maps a manylinux tag string to the corresponding `pkgs.pythonManylinuxPackages.*` compatibility-shim package, added as a `buildInputs` entry, plus wires in `autoPatchelfHook` so the prebuilt wheel's shared-library `RPATH`s get rewritten to point at the Nix store instead of the FHS paths they were built against (`mk-poetry-dep.nix` line 131).

Net effect: there is no cross-building of multiple wheel variants — a given Nix evaluation for a given `python`/platform combination resolves to exactly one file per package (§5.2), decided at eval time from whatever the lockfile happened to record for that platform. Building the same project for a different platform means re-evaluating with a different `python`/`pkgs`, not multiplexing within one derivation.

---

## 8. Why this tool exists: the actual gap between Poetry/pip and Nix

Concretely, from what was read:

- **Network access vs. sandboxing.** `pip`/`poetry install` normally resolve *and* download from PyPI (or a private index) at install time, with full network access. Nix sandboxes derivation builds with no network access except inside a declared fixed-output derivation. `poetry2nix` resolves this by moving every network fetch (§5) *out* of the build derivations and into `fetchFromPypi`/`fetchurl`/`fetchGit`-style fixed-output derivations, each individually hash-pinned from `poetry.lock`, then feeding `pipBuildHook`'s `pip wheel --no-index --no-deps --no-build-isolation` (§6.1) only already-fetched, already-in-the-Nix-store sources. This is the same shape of problem `DEPS.md` §8-9 describes for Gossamer (fetch-then-verify-then-build, cache keyed by content digest), solved with Nix's native fixed-output-derivation primitive rather than a bespoke fetch/cache/vendor pipeline the way `gossamer-pkg` builds one.
- **PEP 517 build-backend diversity is the dominant source of complexity, not fetching.** Fetching+hashing is a solved, mechanical problem (§5) — the actual bulk of the codebase (~4,500-line `overrides/default.nix`, the four dependency-stripping setup hooks, the Cargo-hash sidecar table) exists because `pip wheel` still needs a working, sandboxed instance of *whatever build backend that specific package declares* (`setuptools`, `hatchling`, `flit-core`, `maturin`/Cargo, `pdm-backend`, raw `distutils`, ...), and PyPI package metadata routinely omits build-time requirements that install-time metadata doesn't need to declare (`docs/edgecases.md`'s `ModuleNotFoundError: No module named 'setuptools'` case is the canonical example, reproduced with a full build log in that doc). None of this is expressible from `pyproject.toml`/`poetry.lock` alone; it has to be encoded as external, hand-maintained Nix knowledge.
- **Native/compiled dependencies are a second, orthogonal axis.** Packages like `numpy`/`scipy`/`psycopg2`/`gdal` need real system libraries (BLAS/LAPACK, `libpq`, GDAL's C++ libraries, a Fortran compiler) that exist completely outside the Python packaging metadata model and must be supplied as Nix `buildInputs` by hand, per package (§6.2).
- **Dependency resolution genuinely isn't part of the Nix-side problem** — see the introduction: `poetry2nix` intentionally does none of it, trusting `poetry.lock` as ground truth. The "why does this tool exist" question therefore reduces almost entirely to "how do I turn an already-resolved, already-hash-pinned dependency graph into buildable, sandboxed, cacheable Nix derivations," not "how do I pick compatible versions."

---

## 9. Limitations and gotchas (documented, not inferred)

From `docs/edgecases.md` (read in full) and issue #1865:

- **Missing declared build-system deps** (§6.1/6.2) is described as the most common failure mode, requiring a manual `overridePythonAttrs` per package to add the missing `nativeBuildInputs`/`buildInputs`.
- **Overrides silently supersede the lockfile.** `docs/edgecases.md`'s "Overriding package versions" section demonstrates, with a worked example, that if a user's override changes a package's `version`/`src`, "the poetry application will build successfully **but** the version that is selected ... is 2.0 although `poetry.lock` says 1.0" — i.e. there is no drift check analogous to Gossamer's `--locked` (`DEPS.md` §7) between what the overlay ends up building and what the lockfile says was resolved. Nothing in `poetry2nix` re-verifies consistency after overrides apply.
- **`extras`-with-shared-dependency infinite recursion**: documented example is `dask[distributed]`, where `distributed` transitively depends on `dask` again, producing a Nix "infinite recursion" evaluation error; documented workaround is installing the two packages separately rather than via the extra.
- **Rust/Cargo extensions need a second, separately-maintained hash table** (`getCargoHash`, §6.3), with "still needs human-in-a-loop" acknowledged directly in the docs; `preferWheels = true` is the documented escape hatch, explicitly flagged as trading reproducible-source builds for prebuilt-wheel supply-chain trust.
- **Editable installs** (`mkPoetryEditablePackage`, `editable.nix`) are a separate, minimal code path — a `.pth` file plus a synthesized `.egg-info`/`PKG-INFO` — not the same machinery used for locked packages; only wired up automatically for `path`-source dependencies with `develop = true` (`mkPoetryEnv`, `default.nix` lines 324–341).
- **Maintainer's own retrospective** (issue #1865, quoted at length because it is unusually direct architectural criticism from the original author, useful as a checklist of what *not* to repeat):
  > - Dependency solving isn't very good ... but with `uv` gaining a lock file I don't see the point any more.
  > - Nixpkgs Python builders aren't good. They should be replaced by [pyproject.nix's build approach].
  > - The whole API is misdesigned. `mkPoetry*` shouldn't exist. A python2nix tool should generate an overlay to use with environment composition primitives ... Having a default choice between `sdist` or `wheel` was a mistake. Users should be responsible for making this choice themselves as it shapes your entire user experience.
  > - Overrides ... is my biggest annoyance in regards to maintenance ... I don't think python2nix tooling should come with overrides that taper over lock file deficiencies. Overrides are ephemeral in nature and bit-rot. It's better [for] external projects to take on this role, and let python2nix tooling focus on getting the semantics right.

  Every one of these four complaints maps onto a concrete, load-bearing design choice documented above (§4's `mkPoetry*`-shaped API, §6.2's in-tree override table, §7's fixed sdist-vs-wheel preference). Whatever `gossamer2nix` decides here should be a deliberate choice made with this critique in view, not a default fallen into the same way.

---

## Input → Output summary

**Input** (entirely produced by upstream Poetry tooling, never by `poetry2nix`):

- `pyproject.toml`: project metadata, PEP 508-style dependency/extras declarations (constraint/marker text only — not used to pick versions), and `build-system.requires` (the project's own PEP 517 backend).
- `poetry.lock`: the fully-resolved dependency graph as `[[package]]` entries — `name`, `version`, `source` (registry/git/url/directory/file), per-file `hash`(es) in `sha256:<hex>` form, transitive `dependencies` with their own markers, `python-versions`. **Trusted verbatim, as ground truth — never re-resolved, and its `content-hash` staleness field is never checked.**

**Output**: a set of nixpkgs-shaped Python package derivations, produced as an **overlay** (`python.override { packageOverrides }`) over the target `python.pkgs` set — one `buildPythonPackage` derivation per compatible locked package, each with its `src` a fixed-output derivation (`fetchFromPypi`/`fetchurl`/`fetchGit`) whose `outputHash` is the lockfile's `hash` field reused as-is (no format translation needed, since Poetry's `sha256:<hex>` already matches Nix's own hash grammar), composed with a large hand-maintained override overlay (`defaultPoetryOverrides`) supplying the non-lockfile-expressible bits: missing build-system inputs, native/system library `buildInputs`, and per-platform wheel selection driven off `pkgs.stdenv.targetPlatform`.

**The single most load-bearing insight for a `gossamer2nix` design**: `poetry2nix` cleanly separates "replay an already-resolved lock as pinned Nix fetches" (mechanical, ~fully solved, and the part that most resembles what `gossamer2nix` needs to do for Gossamer's registry/git/tarball deps per `DEPS.md` §7-9) from "actually build each dependency inside a sandbox" (the part that consumed the overwhelming majority of the project's ~4,500 lines of override code and, per the maintainer, was never solved well). Gossamer's simpler, single-toolchain build model (`gos build`, no PEP 517-style backend zoo) suggests `gossamer2nix` may be able to avoid `poetry2nix`'s single biggest cost center — but should not assume this without first re-confirming, per `DEPS.md` §12, that non-`path` dependencies are even wired into `gos build` at all; if/when they are, the open question worth tracking is whether Gossamer registry packages ever need anything resembling Poetry's build-backend variance (e.g. its own `[rust-bindings]` FFI path, `DEPS.md` §3.1) badly enough to reproduce even a small slice of this problem.

---

## Sources

- [`nix-community/poetry2nix`](https://github.com/nix-community/poetry2nix), `master` @ `ce2369d` (2025-04-03, confirmed current as of 2026-07-26 via `gh api`), read via local shallow `git clone`:
  - `default.nix`, `lib.nix`, `mk-poetry-dep.nix`, `editable.nix`, `cli.nix`, `overlay.nix`, `plugins.nix`, `shell-scripts.nix`
  - `fetchers/default.nix`, `fetchers/fetch-from-pypi.sh`
  - `hooks/default.nix`, `hooks/pip-build-hook.sh`
  - `overrides/default.nix` (~4,521 lines; `numpy`, `psycopg2`/`psycopg2-binary`/`psycopg2cffi`, `pemja` entries read directly)
  - `vendor/pyproject.nix/lib/pypa.nix` (vendored subset of [`pyproject-nix/pyproject.nix`](https://pyproject-nix.github.io/pyproject.nix/))
  - `docs/edgecases.md` (read in full)
  - `README.md` (maintenance status, quickstart, API reference sections)
  - `tests/trivial/poetry.lock`, `tests/trivial-poetry-1_2_0/poetry.lock` (lockfile format examples)
- [`nix-community/poetry2nix` issue #1865](https://github.com/nix-community/poetry2nix/issues/1865) ("Maintenance: Looking for maintainers"), full body read via `gh api repos/nix-community/poetry2nix/issues/1865`
- `gh api repos/nix-community/poetry2nix` and `.../commits` — repo metadata (`archived: false`, `fork: false`) and commit history, used to confirm current maintenance/activity status as of 2026-07-26

**This reflects one point-in-time reading of a specific `master` commit and may drift** if the repo gains new maintainers (per the open issue, actively solicited). Re-verify before depending on exact line numbers or file contents.
