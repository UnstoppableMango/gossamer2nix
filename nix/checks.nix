{
  buildGossamerApplication,
  gossamer,
  runCommand,
  craneLib,
  src,
  cargoArtifacts,
}:

{
  hello-app = buildGossamerApplication {
    pname = "hello";
    version = "0.1.0";
    src = runCommand "gen-src" { } ''
      ${gossamer}/bin/gos new example.com/hello --path $out
    '';
  };

  gossamer2nix-tests = craneLib.cargoTest {
    inherit cargoArtifacts src;
  };
}
