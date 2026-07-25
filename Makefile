build:
	nix build .#

generate gen: nix/checks/hello

update:
	nix flake update

check lint:
	nix flake check

format fmt:
	nix fmt

.PHONY: nix/checks/hello
nix/checks/hello:
	nix build .#checks.hello-app.gen-src --out-link $@
