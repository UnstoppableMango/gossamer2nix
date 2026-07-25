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
	rm -rf $@
	cp -r --no-preserve=mode,ownership "$$(nix build .#hello-gen-src --no-link --print-out-paths)" $@
	chmod -R u+w $@
