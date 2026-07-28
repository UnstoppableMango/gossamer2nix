# Scaffolding for future `[rust-bindings]` (FFI) support — GOALS.md,
# docs/deps/DESIGN.md §4. No Gossamer project declares `[rust-bindings]`
# yet, so nothing calls this yet.
{
  craneLib,
  rustToolchain,
}:

craneLib.overrideToolchain rustToolchain
