{
  pkgs ? import <nixpkgs> { },
}:

{
  buildGossamerApplication = pkgs.callPackage ./builder.nix { };
}
