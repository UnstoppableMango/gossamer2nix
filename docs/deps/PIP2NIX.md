# Dependency Management Bridge: `pip2nix`

Read 2026-07-26 (`master`, HEAD `948d568`, 2025-07-16) as prior-art survey for `gossamer2nix`. Sources: `README.rst`, `CHANGELOG.rst`, `pip2nix/{cli,generate,confspec.ini}`, `pip2nix/models/{package,egg_writer}.py`, `default.nix`, `flake.nix`, a generated sample `python-packages.nix`.

**Key finding up front**: `pip2nix` is not a lockfile translator — because pip's native `requirements.txt` format has no lockfile to translate, `pip2nix` embeds a large chunk of pip's own internal resolver machinery (imported directly from `pip._internal`) and drives it against the live network at generation time, using `nix-prefetch-url`/`-git`/`-hg` to compute Nix hashes as a side effect. There is no separate "lock" step and no offline/sandboxed generation mode — the opposite end of the spectrum from a lockfile-translator like `poetry2nix`.

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/pip2nix](https://github.com/nix-community/pip2nix) (`master`) |
| **Ecosystem** | Python / plain pip (`requirements.txt`) |
| **Maintenance status** | Unhealthy — not archived, but low/reactive activity, `flake.nix` still pinned to `nixos-20.09`, 33 open issues back to 2022, `generate.py` is a thicket of version-compatibility fallback chains against pip's own unstable internals |
| **Ecosystem input** | `requirements.txt` — name + optional version specifier, **no native hash or full transitive graph** |
| **Generated output** | `python-packages.nix` — an overlay, which also doubles as the de facto lockfile (no separate lock artifact exists) |
| **Hash strategy** | Fabricated — no ecosystem hash exists to reuse; live `nix-prefetch-url`/`-git`/`-hg` calls at generation time, trust-on-first-use |
| **Nix build mechanism** | Per-package `super.buildPythonPackage` overlay entries; resolution itself is delegated to pip's own real (internal, unversioned) resolver |

## Input

`pip2nix generate -r requirements.txt` accepts the same CLI surface as `pip install` (`-c` constraints, `-e` editables, `--index-url`, etc.), configurable via `pip2nix.ini`. Critically, this input format carries no reproducibility guarantee: a line like `requests==2.32.4` pins a version string but no content hash and says nothing about the transitive graph — resolving that is pip's job, done fresh against whatever index state exists right now. Unlike `pip-compile --generate-hashes` output or Poetry's lock, a bare `requirements.txt` was never meant to be reproducible on its own, and `pip2nix` doesn't require a hash-pinned/frozen variant — it does all resolution and hashing itself.

## Resolution: reusing pip's own resolver, live

`pip2nix/generate.py` defines `NixFreezeCommand`, a subclass of pip's own `InstallCommand`, with installation itself suppressed (`no_install = True`). Rather than reimplementing resolution, it reconstructs pip's actual internal install pipeline by hand — builds a `PackageFinder`, a `RequirementSet`, a `RequirementPreparer`, and pip's own `Resolver` (imported via a cascading `try/except ImportError` across several historical pip internal module paths) — then calls `resolver.resolve(...)`, so its output is guaranteed to match what `pip install` would actually have installed on that machine at that moment. Consequences: **network access is required at generation time**, unsandboxed, and **resolution is not deterministic across time** — re-running against the same `requirements.txt` later can produce a different output as the index gains new releases, since there's no separate "resolve once, freeze it" step distinct from "generate the Nix expression." `pip2nix` is the only tool surveyed here that skips implementing resolution logic entirely by borrowing the real package manager's resolver wholesale — at the cost of depending on that resolver's internal, explicitly unversioned API.

## Output and hashing: live `nix-prefetch-*`, trust-on-first-use

Once pip's resolver has determined the final package set and each download link, `link_to_nix()` converts each into a Nix fetcher call by shelling out to Nix's own prefetch tools at generation time:

```python
hash = prefetch_url(link.url_without_fragment)        # -> nix-prefetch-url
hash, revision = prefetch_git(url, branch)             # -> nix-prefetch-git
hash, revision = prefetch_hg(url, branch)               # -> nix-prefetch-hg
```

This requires live, unsandboxed network access and the `nix-prefetch-*` scripts on `PATH` at generation time — nothing about this step could run inside a Nix build sandbox; it's a pre-build, developer/CI-time step, after which only the *output* file is consumed by a real sandboxed build. One local optimization: before regenerating, previous output is regex-scanned for existing `url = ...; sha256 = ...;` pairs so unchanged packages skip re-prefetching — a best-effort memoization, not a structured or shareable lockfile. Local `file://`/path links pass through with no fetch or hash at all, analogous to Gossamer's unhashed `path` dependency kind.

The result (`python-packages.nix`) is a `{ pkgs, fetchurl, fetchgit, fetchhg }: self: super: { ... }` overlay — one `super.buildPythonPackage` per resolved package, intra-graph deps wired through `self.<pkg>`, standard nixpkgs overlay idiom. There is no separate machine-consumable lockfile distinct from this file; it is simultaneously the build recipe and the only record of exactly which version+hash was resolved.

## Why the tool exists — the gap vs. a lockfile-based ecosystem

| | Poetry (`poetry.lock`) | plain pip (`requirements.txt`) |
|---|---|---|
| Resolved graph + hashes committed to VCS | Yes | No |
| Deterministic across re-resolution | Yes | No |
| What the `*2nix` tool must do | **Translate** an already-resolved, already-hashed lock | **Resolve, fetch, and hash from scratch, live**, because nothing upstream ever did it |

The entire reason `pip2nix` exists, and the entire reason it's structurally more fragile than `poetry2nix`, is that pip's native input is a constraint list, not a lockfile — `pip2nix` has to manufacture the equivalent of one itself, on every run, by running a real piece of the package manager against the live network, rather than mechanically translating a static file. `poetry2nix` (and a `gossamer2nix` built against Gossamer's own `project.lock`) gets to skip this entire problem for free, because the ecosystem-native tool already did the resolution-and-hashing work and published a stable, checked-in result.

## Relevance to `gossamer2nix`

A tool whose entire hashing/fetching pipeline is built by importing another tool's internal, unversioned API surface is fragile in a way that's largely avoidable if Gossamer's own resolver/lockfile (`../DEPS.md` §6-7) stays stable and public. This is the strongest argument in this whole survey for `gossamer2nix` consuming Gossamer's own `project.lock` directly — as `gomod2nix`/`poetry2nix` do with their respective ecosystem-native lockfiles — rather than re-deriving a resolution the way `pip2nix` is forced to.
