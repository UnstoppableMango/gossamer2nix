{
  buildGossamerApplication,
  gossamer,
  runCommand,
}:

{
  hello-app = buildGossamerApplication {
    pname = "hello";
    version = "0.1.0";
    src = runCommand "gen-src" { } ''
      ${gossamer}/bin/gos new example.com/hello --path $out
    '';
  };
}
