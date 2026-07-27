# Dependency Management in `gomod2nix`

This document was compiled by reading source directly from [nix-community/gomod2nix](https://github.com/nix-community/gomod2nix) (`master` branch, as of 2026-07-26) — specifically `README.md`, `docs/getting-started.md`, `internal/generate/generate.go`, `internal/schema/schema.go`, `internal/cmd/root.go`, `builder/default.nix`, `builder/fetch.sh`, `builder/parser.nix`, `builder/hooks/default.nix`, `overlay.nix`, and the example `gomod2nix.toml` checked into that repo.
It exists to ground the design of this repo's `gossamer2nix` deps-lock/Nix adapter (see [GOALS.md](../../GOALS.md)) in `gomod2nix`'s real implementation, as the direct prior art this project is explicitly modeled on ("in the spirit of `gomod2nix`" — repo README/CLAUDE.md).

**Repo identity check**: `nix-community/gomod2nix` is the canonical/maintained repo (GitHub org description: "Convert applications using Go modules to Nix expressions [maintainer=@marcusramberg]"). A `kwohlfahrt/gomod2nix` fork exists but was not the subject of this research. Default branch is `master`, not `main`.

**This reflects one point-in-time reading of upstream `master` and may drift.** Re-verify struct/field names against current upstream source before depending on exact details in code.

---

## TL;DR: Input → Output

| | Ecosystem-native input | Nix-consumable output |
|---|---|---|
| **Read** | `go.mod` (parsed via `golang.org/x/mod/modfile`), plus whatever `go mod download --json` and `go list -mod=readonly -f {{.ImportPath}} all` report after consulting the ambient Go toolchain/module cache/proxy | — |
| **Written** | — | `gomod2nix.toml` (schema v3): `[mod."<module path>"]` → `version`, `hash` (Nix SRI `sha256-<base64>`), optional `replaced`; plus top-level `subPackages`, `goPackagePath`, `cachePackages` |
| **Key transform** | Go's `go.sum` `h1:` dirhash (SHA-256 over a sorted list of per-file SHA-256 digests, [`golang.org/x/mod/sumdb/dirhash`](https://pkg.go.dev/golang.org/x/mod/sumdb/dirhash) format, base64-std) | **Not reused.** `gomod2nix` independently NAR-serializes the *already-downloaded* module directory (`nar.DumpPathFilter`, filtering out `.DS_Store`) and SHA-256s that byte stream, producing a hash in the same shape Nix's own `outputHash`/fixed-output-derivation mechanism expects (recursive/NAR mode). `go.sum` is **never read** by the generator at all — the tool doesn't parse it, doesn't compare against it, doesn't reuse its digests. |

The single most important fact for a mirror-image `gossamer2nix` design: **the lockfile hash is *not* a translation of the ecosystem's own checksum format.** It is a fresh hash computed by `gomod2nix` itself, over the *result of actually fetching the dependency*, serialized the way Nix serializes directories (NAR) rather than the way Go serializes them (`dirhash`'s sorted-file-list scheme). The ecosystem checksum (`go.sum`) is left doing its native job (protecting `go mod download` against tampering/inconsistency via `GOSUMDB`) and is completely orthogonal to what ends up in `gomod2nix.toml`.

---

## 1. What input does `gomod2nix` consume?

Entry point: `gomod2nix generate` (also the default no-subcommand behavior — `internal/cmd/root.go`, `rootCmd.Run = generateFunc`).

`internal/generate/generate.go`, function `common(directory)`:

1. Reads `<directory>/go.mod` off disk (`os.ReadFile`) and parses it with `golang.org/x/mod/modfile.Parse` — the same parser the Go toolchain itself uses, not a hand-rolled TOML/regex reader. From the parsed `modfile.File` it builds a `replace` map: `new-path → old-path`, i.e. it inverts each `replace` directive's target back to its source module path (used later to know which cache key/lockfile entry a replaced dependency's fetched content should be filed under).
2. Shells out to **`go mod download --json`** (`cmd.Dir = directory`), streaming newline-delimited JSON objects (Go's own `go help mod download` JSON schema) decoded into a local `goModDownload` struct with fields `Path`, `Version`, `Info`, `GoMod`, `Zip`, `Dir`, `Sum`, `GoModSum`. This is the *only* thing that talks to the network/module proxy, and it is entirely Go's own tooling — `gomod2nix` does not implement the Go module proxy protocol itself.
   - `Dir` is the critical field: the local filesystem path (inside `$GOPATH/pkg/mod` or `$GOMODCACHE`) where `go mod download` extracted that module's verified, unpacked source tree.
   - `Sum`/`GoModSum` (the `h1:` values, i.e. what's also recorded in `go.sum`) are present in the struct but **never read or used anywhere else in `generate.go`** — confirmed by reading the whole file. They pass through Go's own module verification (against `GOSUMDB`/local `go.sum`) as part of `go mod download` itself, but `gomod2nix` doesn't propagate or re-encode them.
3. For the optional `--with-deps` cache-priming path (`GenerateCacheDeps`), it separately re-parses `go.mod` (for the current module's own path, to exclude self-packages) and runs **`go list -mod=readonly -f "{{.ImportPath}}" all`**, then filters out `std`, the current module's own packages, `vendor/...` entries, and anything containing `/internal`, keeping only paths containing a `.` (i.e. external, non-stdlib import paths) — this becomes `cachePackages` in the output.

So: **exactly two ecosystem-native files/commands are the real input surface** — `go.mod` (parsed structurally) and the live output of `go mod download --json` / `go list ... all` (which themselves consult `go.sum`, `GOPROXY`, `GOSUMDB`, and the local module cache, but none of *that* machinery is reimplemented by `gomod2nix` — it's delegated wholesale to the `go` binary). `go.sum` itself, `vendor/modules.txt`, and `go.work` are not parsed by the generator at all (see §5, Limitations).

---

## 2. What does it produce? Exact schema

`internal/schema/schema.go`, `SchemaVersion = 3`:

```go
type Package struct {
    GoPackagePath string `toml:"-"`               // the TOML *key*, not a field
    Version       string `toml:"version"`
    Hash          string `toml:"hash"`
    ReplacedPath  string `toml:"replaced,omitempty"`
}

type Output struct {
    SchemaVersion int                 `toml:"schema"`
    Mod           map[string]*Package `toml:"mod"`
    SubPackages   []string `toml:"subPackages,omitempty"`
    GoPackagePath string   `toml:"goPackagePath,omitempty"`
    CachePackages []string `toml:"cachePackages,multiline,omitempty"`
}
```

Rendered (real example, `gomod2nix.toml` from the `gomod2nix` repo's own root, fetched 2026-07-26):

```toml
schema = 3

[mod]

[mod.'github.com/spf13/cobra']
version = 'v1.10.1'
hash = 'sha256-OP6wdqk4dvBD8U5aicTkySHZ2s0LWnBo2TST2SmgcpM='

[mod.'golang.org/x/tools/go/vcs']
version = 'v0.1.0-deprecated'
hash = 'sha256-57YB10tiRsVSRvJqKYST+iON6yZYGL7eRSzrFcImC8Y='
```

Field notes:
- **`hash`** is Nix's SRI-style string: `"sha256-" + base64.StdEncoding(digest)` — i.e. `sha256-<standard-base64>`, *not* base32 and *not* the `nix-base32` alphabet Nix uses in some legacy contexts. This exact shape is what modern Nix (`outputHash`, `fetchurl`, etc.) accepts as an SRI hash string.
- **`replaced`** is populated only when the module was reached via a `go.mod` `replace` directive pointing at another *module-path* replacement (not a local-path replacement — those are handled differently at build time, see §3/§4). It records the *original* (pre-replace) path so the fetched content is still keyed/labeled sensibly.
- **`subPackages`**/`goPackagePath` are populated only by the `gomod2nix generate <import-paths...>` form (installing specific packages from a module without a local checkout, `internal/generate/temp.go`) — irrelevant to the common "have a repo, run `gomod2nix`" flow.
- **`cachePackages`** is populated only by `--with-deps`, a build-cache-priming optimization, not part of core dependency resolution.
- The map is written with `SetIndentTables(true)` via `github.com/pelletier/go-toml/v2`; the generator explicitly `sort.Slice`s the underlying `[]*Package` by `GoPackagePath` before marshaling, so output is deterministic across runs given identical input.
- **Incremental regeneration**: `schema.ReadCache(goMod2NixPath)` reads any *existing* `gomod2nix.toml` first; if a module's path+version already has a cached hash, `generate.go`'s `GeneratePkgs` reuses it instead of re-fetching+re-hashing (`cached.Version == dl.Version` check) — an optimization to avoid re-hashing unchanged deps on every regeneration, not a correctness mechanism.

---

## 3. The hash computation — how `go.sum` checksums become Nix hashes

This is the core mechanism and the part most relevant to `gossamer2nix`, in `GeneratePkgs` (`internal/generate/generate.go`):

```go
h := sha256.New()
err := nar.DumpPathFilter(h, dl.Dir, sourceFilter)   // nar = github.com/nix-community/go-nix/pkg/nar
digest := h.Sum(nil)

pkg := &schema.Package{
    GoPackagePath: goPackagePath,
    Version:       dl.Version,
    Hash:          "sha256-" + base64.StdEncoding.EncodeToString(digest),
}
```

where `sourceFilter` excludes only `.DS_Store` files (case-insensitive basename match) and `dl.Dir` is the path `go mod download --json` reported as the already-fetched, already-verified (by Go's own `go.sum`/`GOSUMDB` machinery) module source tree sitting in the local Go module cache.

`nar.DumpPathFilter` serializes that directory tree using **NAR** (Nix ARchive) format — the same canonical, order-independent, metadata-normalized directory-serialization format Nix itself uses internally for `outputHashMode = "recursive"` fixed-output derivations and for `nix-store --dump`/store-path content-addressing. So the hash written into `gomod2nix.toml` is *literally* "the NAR hash Nix would compute if it dumped this exact directory" — computed once, up front, by the `gomod2nix` CLI (which trusts the ambient `go` toolchain's own verification), then handed to Nix later as a promise (§4).

**Confirmed: `go.sum`'s `h1:` values are read into the `Sum`/`GoModSum` struct fields from `go mod download --json`'s output but are never used to produce, validate, or cross-check the `hash` field.** The two hashes protect different things and use incompatible serialization schemes (Go's `dirhash` hashes a manifest of per-file digests sorted by name; NAR is a full recursive binary directory encoding), so there is no direct arithmetic relationship — `gomod2nix` fully discards the ecosystem checksum format and replaces it with a Nix-native one derived empirically by fetching once.

---

## 4. The Nix-side mechanism: per-module fixed-output derivations

`builder/default.nix` + `builder/fetch.sh`:

```nix
fetchGoModule = { hash, goPackagePath, version, go }:
  stdenvNoCC.mkDerivation {
    name = "${baseNameOf goPackagePath}_${version}";
    builder = ./fetch.sh;
    inherit goPackagePath version;
    nativeBuildInputs = [ cacert git go jq ];
    outputHashMode = "recursive";
    outputHashAlgo = null;      # null because `hash` is already a self-describing SRI string
    outputHash = hash;          # <- straight from gomod2nix.toml's `hash` field
    impureEnvVars = fetchers.proxyImpureEnvVars ++ [ "GOPROXY" ];
  };
```

`builder/fetch.sh` (full script):

```bash
source $stdenv/setup
export HOME=$(mktemp -d)
# Call once first outside of subshell for better error reporting
go mod download "$goPackagePath@$version"
dir=$(go mod download --json "$goPackagePath@$version" | jq -r .Dir)
chmod -R +w $dir
find $dir -iname ".ds_store" | xargs -r rm -rf
cp -r $dir $out
```

So **one fixed-output derivation (FOD) per Go module** (not one big vendor-directory FOD, and not `buildGoModule`'s single-`vendorHash` approach): each module gets its own derivation that (a) is allowed network access — Nix's sandbox exception for FODs, gated by `impureEnvVars` including `GOPROXY` and standard proxy vars — (b) re-runs `go mod download <module>@<version>` itself inside the sandboxed build, (c) copies the resulting directory to `$out`, and (d) Nix then hashes `$out` (NAR mode) and compares against the declared `outputHash`. If they don't match, the build fails — this is what actually enforces reproducibility; `gomod2nix.toml`'s `hash` is not "trusted," it's *checked* by Nix itself on every build.

`mkVendorEnv` (also `builder/default.nix`) then:
- Maps every `[mod.*]` entry through `fetchGoModule`, producing one derivation per dependency (`sources = mapAttrs (goPackagePath: meta: fetchGoModule {...}) modulesStruct.mod`).
- Assembles a `vendor/` tree by symlinking (`internal.symlink`, a small Go program built at Nix-eval time from `builder/symlink/symlink.go`) each fetched module into the right `vendor/<module-path>` location — i.e. it reconstructs Go's own `vendor/` convention purely from Nix-fetched pieces, rather than shipping/trusting a `vendor/` directory from the source repo.
- Handles *local-path* `replace` directives (`replace foo => ../bar`) separately by symlinking straight to the given relative path (`pwd + "/${value.path}"`) — these have no hash and are not fetched, matching path-dependency semantics generally (no digest pin, direct filesystem reference).

`mkGoEnv` / `buildGoApplication` (also `builder/default.nix`) then build against that reconstructed `vendor/` tree via custom Nix setup hooks (`goConfigHook`/`goBuildHook`/`goCheckHook`/`goInstallHook`, `builder/hooks/default.nix`) instead of letting `go build` reach out to any module proxy — this is the actual mechanism by which the final application build is fully offline/sandboxed even though Go's native tooling assumes network access by default. `builder/parser.nix` is a separate, pure-Nix `go.mod` parser (regex/fold-based, no Go toolchain needed) used purely to extract `replace`/`require` info at Nix-evaluation time — a second, independent `go.mod` parser from the one in `internal/generate/generate.go` (which uses Go's real `modfile` package), because the Go-based generator and the Nix-based builder are separate processes that both need to understand `go.mod`.

**Why this design (the gap being closed):** `go build`/`go mod download` want to talk to a module proxy over the network, but a Nix build (outside of an explicit FOD) is sandboxed with no network access, for reproducibility. Go's own checksum (`go.sum`'s `h1:` dirhash) isn't something Nix's `outputHash` mechanism can consume directly (different serialization, and Nix needs a hash that matches *its own* content-addressing scheme, NAR, to make the FOD's output verifiably equal to what building it again would produce). `gomod2nix` bridges the gap by: (1) doing one honest, network-connected fetch+verify per module *ahead of time* (trusting Go's own `go.sum`/`GOSUMDB` machinery for authenticity at that stage), (2) re-hashing the result in Nix's own format, (3) recording that in a lockfile, and (4) at actual build time, re-fetching each module inside a per-module FOD (again over the network, since FODs are the sandbox's one network-permitted case) and letting Nix's own hash check enforce that the second fetch produced byte-identical content to the first. Ordinary non-FOD build steps (the actual `go build`/`go install` of the application) then run fully offline against the locally-assembled `vendor/` tree.

---

## 5. Known limitations / gotchas

- **`go.work` / Go workspace mode is not supported.** Per a NixOS Discourse thread ("Gomod2nix with go workspaces") and corroborating discussion on a related `nixpkgs` issue (`buildGoModule: support Go workspaces`, NixOS/nixpkgs#203039): neither `buildGoModule` nor `gomod2nix` can build multi-module `go.work` monorepos out of the box — `generate.go` only ever reads a single `go.mod` in the given `directory`, with no `go.work` parsing anywhere in the read source. **Unconfirmed beyond forum discussion** — no explicit "not supported" statement was found in gomod2nix's own docs/issues during this pass; treat as a strong signal, not a certainty, and re-check current issues before relying on it.
- **`vendor/modules.txt` is not read.** The generator always calls `go mod download`/`go list` against the module cache/proxy; it does not parse an existing `vendor/` directory as an input. (It *writes* a synthetic `vendor/` tree at build time from Nix-fetched pieces — see §4 — which is a different thing from reading a repo-committed `vendor/modules.txt`.)
- **`replace` directives**: module-path replacements (`replace old => new@version`) are fully handled — fetched and hashed like any other dependency, with `replaced` recording the pre-replace path (§2). Local-path replacements (`replace old => ../local/dir`) bypass fetching/hashing entirely and are symlinked directly at build time (§4) — meaning they are **not reproducible/pinned** in the same sense as everything else (same category of gap as Gossamer's own `{ path = ... }` dependencies per `DEPS.md` §3, §7).
- **Private modules**: not specially handled by `gomod2nix` itself — it delegates entirely to the ambient `go` toolchain's own auth mechanisms (`GOPRIVATE`, `.netrc`, `GOPROXY` overrides, SSH config) both when `generate` first fetches+hashes (needs credentials on the machine running `gomod2nix generate`) and again inside each per-module FOD at build time (`fetch.sh` includes `git` and `cacert` in `nativeBuildInputs`, and `impureEnvVars` passes through proxy env vars — but nothing that looks like injected auth for private registries was found in the read source; **unconfirmed** whether/how credentials are meant to reach the sandboxed FOD in CI-style hermetic setups).
- **Cache/incremental generation correctness** depends on trusting a previously-generated `gomod2nix.toml`'s hash for unchanged `(path, version)` pairs (§2) rather than re-verifying it — a deliberate performance/trust tradeoff, not a bug, but worth naming since a `gossamer2nix` equivalent should decide the same tradeoff deliberately.
- **`.DS_Store` filtering** is the *only* source-tree filtering applied before hashing (`sourceFilter` in `generate.go`, mirrored in `fetch.sh`'s `find $dir -iname ".ds_store" | xargs -r rm -rf`) — both sides of the pin/verify pair apply the identical filter, which is precisely what keeps the two independently-computed NAR hashes (generation-time vs. build-time) equal. **A gossamer2nix analog must apply an identical filter on both the generation and fetch sides, or hashes will never match.**

---

## Sources

- `README.md`, `docs/getting-started.md`, `overlay.nix` — top-level usage, flake/niv setup, `gomod2nix generate`/`gomod2nix import` CLI surface.
- `internal/cmd/root.go` — CLI wiring (`generate`, `import` subcommands, flags: `--dir`, `--outdir`, `--jobs`, `--with-deps`).
- `internal/generate/generate.go` — `go.mod` parsing, `go mod download --json` / `go list ... all` invocation, NAR-hash computation (`nar.DumpPathFilter` + SHA-256 + base64), cache-reuse logic.
- `internal/schema/schema.go` — `Package`/`Output` struct definitions, `SchemaVersion = 3`, TOML marshal/unmarshal, `ReadCache`.
- `builder/default.nix` — `fetchGoModule` (per-module fixed-output derivation), `mkVendorEnv`, `mkGoEnv`, `mkGoCacheEnv`, `buildGoApplication`, Go-version selection from `go.mod`.
- `builder/fetch.sh` — the actual per-module fetch builder script run inside each FOD.
- `builder/parser.nix` — pure-Nix `go.mod` parser used at build time (for `replace`/`require` directives), independent of the Go-based generator.
- `builder/hooks/default.nix` — `goConfigHook`/`goBuildHook`/`goCheckHook`/`goInstallHook` setup hooks.
- `gomod2nix.toml` (repo root, self-hosted example) — concrete schema-v3 output sample.
- [nix-community/gomod2nix](https://github.com/nix-community/gomod2nix) GitHub repo description, confirming canonical/maintained status and current maintainer.
- [NixOS Discourse: "Gomod2nix with go workspaces"](https://discourse.nixos.org/t/gomod2nix-with-go-workspaces/43134) and [NixOS/nixpkgs#203039](https://github.com/NixOS/nixpkgs/issues/203039) — workspace-mode limitation (flagged unconfirmed above).
- [Tweag announcement blog post](https://www.tweag.io/blog/2021-03-04-gomod2nix/) — referenced by the README for motivation/design comparisons; not independently re-verified in this pass.

Read via GitHub raw content (`raw.githubusercontent.com/nix-community/gomod2nix/master/...`) and the GitHub contents API on 2026-07-26; no local clone of `gomod2nix` exists in this workspace.
