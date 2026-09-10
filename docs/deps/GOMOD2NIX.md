# Dependency Management in `gomod2nix`

`gossamer2nix` is explicitly modeled "in the spirit of `gomod2nix`" — this is the direct prior art. Read 2026-07-26 from `internal/generate/generate.go`, `internal/schema/schema.go`, `internal/cmd/root.go`, `builder/default.nix`, `builder/fetch.sh`, `builder/parser.nix`, `builder/hooks/default.nix`.

## At a glance

| | |
|---|---|
| **Canonical repo** | [nix-community/gomod2nix](https://github.com/nix-community/gomod2nix) (`master`) |
| **Ecosystem** | Go modules |
| **Maintenance status** | Active |
| **Ecosystem input** | `go.mod` (parsed via `x/mod/modfile`) + live `go mod download --json` / `go list ... all` output. **`go.sum` is never read.** |
| **Generated output** | `gomod2nix.toml` (schema v3): `[mod."<path>"]` → `version`, `hash`, optional `replaced` |
| **Hash strategy** | Re-derived. `go.sum`'s `h1:` dirhash is discarded entirely — `gomod2nix` NAR-serializes the already-downloaded module directory itself and SHA-256s that, producing `sha256-<base64>` |
| **Nix build mechanism** | One fixed-output derivation per Go module (`fetchGoModule`), symlink-assembled into a synthetic `vendor/` tree |

## Input

`gomod2nix generate` reads `go.mod` with Go's own `modfile` parser (building a `replace` map), then shells out to `go mod download --json` — the only thing that touches the network/module proxy, entirely delegated to the real `go` binary. The critical field returned is `Dir`: the local path where `go mod download` already extracted and verified that module's source. `Sum`/`GoModSum` (the `h1:` values also written to `go.sum`) are present in the response struct but never used elsewhere — confirmed by reading the whole file. `--with-deps` additionally runs `go list -mod=readonly -f {{.ImportPath}} all` to populate a cache-priming `cachePackages` list.

## Output and the hash computation

```go
h := sha256.New()
nar.DumpPathFilter(h, dl.Dir, sourceFilter)   // NAR-serialize the fetched module dir
digest := h.Sum(nil)
pkg.Hash = "sha256-" + base64.StdEncoding.EncodeToString(digest)
```

`sourceFilter` excludes only `.DS_Store`. `nar.DumpPathFilter` serializes the directory the same way Nix itself does for `outputHashMode = "recursive"` — so the hash written to `gomod2nix.toml` is literally "the NAR hash Nix would compute if it dumped this exact directory," computed once up front by trusting the ambient `go` toolchain's own verification. Go's `dirhash` (a manifest of sorted per-file digests) and Nix's NAR hash protect different things via incompatible serializations, so there is no arithmetic conversion between them — `gomod2nix` fully replaces the ecosystem checksum with a Nix-native one derived empirically from a real fetch.

Output is deterministic (`sort.Slice` by import path before marshaling), and incremental: an existing `gomod2nix.toml`'s hash is reused for unchanged `(path, version)` pairs rather than re-verified — a deliberate performance/trust tradeoff.

## Nix-side mechanism

```nix
fetchGoModule = { hash, goPackagePath, version, go }:
  stdenvNoCC.mkDerivation {
    outputHashMode = "recursive";
    outputHash = hash;   # straight from gomod2nix.toml
    impureEnvVars = fetchers.proxyImpureEnvVars ++ [ "GOPROXY" ];
    builder = ./fetch.sh;   # re-runs `go mod download` inside the sandboxed FOD
  };
```

One FOD per module, each allowed network access (Nix's sandbox exception for FODs) to re-run `go mod download <module>@<version>` itself; Nix then NAR-hashes `$out` and compares against the declared `outputHash`. This is what actually enforces reproducibility — the lockfile's `hash` is checked, not trusted. `mkVendorEnv` then symlinks every fetched module into a reconstructed `vendor/<module-path>` tree; local-path `replace` directives are symlinked directly with no hash/fetch. The final build runs against that `vendor/` tree via custom setup hooks, never touching the network.

## Why the tool exists

`go build`/`go mod download` want network access to a module proxy; a Nix derivation (outside an FOD) is sandboxed. `gomod2nix` bridges the gap by doing one honest, network-connected fetch+verify per module ahead of time (trusting Go's own `go.sum`/`GOSUMDB`), re-hashing the result in Nix's own NAR format, recording that in a lockfile, then at build time re-fetching each module inside a per-module FOD and letting Nix's own hash check enforce that the second fetch matches the first. The actual application build then runs fully offline.

## Limitations

- **`go.work` workspace mode**: no parsing found anywhere in the generator; likely unsupported (forum-corroborated, not confirmed in gomod2nix's own docs).
- **`vendor/modules.txt` is never read** as an input — the generator always talks to the module cache/proxy directly.
- **Local-path `replace` directives** bypass fetching/hashing entirely — not reproducible/pinned, same category of gap as Gossamer's own `{ path = ... }` dependencies.
- **Private modules**: no special handling — delegated entirely to the ambient `go` toolchain's own auth (`GOPRIVATE`, `.netrc`, `GOPROXY`); unconfirmed whether credentials are meant to reach a sandboxed FOD in CI.
- **`.DS_Store` filtering must match exactly** between generation-time and build-time hashing, or the two independently-computed NAR hashes will never agree — a gossamer2nix analog needs the same discipline.
