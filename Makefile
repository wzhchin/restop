.PHONY: help check test fmt-check package

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "%-12s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

check: ## Type-check all targets
	cargo check --all-targets

test: ## Run all tests
	cargo test --all-targets

fmt-check: ## Check Rust formatting
	cargo fmt --all -- --check

package: ## Build the Arch Linux package
	@command -v makepkg >/dev/null 2>&1 || { printf '%s\n' 'makepkg is required to build the Arch Linux package' >&2; exit 1; }
	@mkdir -p build/archlinux
	makepkg --dir packaging/archlinux --cleanbuild --clean --force -p PKGBUILD BUILDDIR="$(CURDIR)/build/archlinux" PKGDEST="$(CURDIR)/build/archlinux"
