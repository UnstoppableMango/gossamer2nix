# Shared craneLib, used to build the gossamer2nix CLI (root Cargo crate)
# and for future `[rust-bindings]` (FFI) support — GOALS.md, docs/deps/DESIGN.md §4.
{
  craneLib,
  rustToolchain,
}:

craneLib.overrideToolchain rustToolchain
