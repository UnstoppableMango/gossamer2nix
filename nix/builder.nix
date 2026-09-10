{
  lib,
  stdenv,
  gossamer,
}:

{
  pname,
  version,
  src,
  # Extra arguments appended to the `gos build` invocation, e.g.
  # [ "--target" "aarch64-unknown-linux-musl" ] or [ "--locked" "--reproducible" ].
  gosBuildFlags ? [ ],
  nativeBuildInputs ? [ ],
  ...
}@args:

stdenv.mkDerivation (
  (builtins.removeAttrs args [ "gosBuildFlags" ])
  // {
    nativeBuildInputs = [ gossamer ] ++ nativeBuildInputs;

    buildPhase =
      args.buildPhase or ''
        runHook preBuild
        gos build --release --out-dir dist ${lib.escapeShellArgs gosBuildFlags}
        runHook postBuild
      '';

    installPhase =
      args.installPhase or ''
        runHook preInstall
        install -Dm755 -t $out/bin dist/*
        runHook postInstall
      '';
  }
)
