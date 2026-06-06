.PHONY: binary deb aur-srcinfo clean help

FEATURES = desktop-ui system-audio
VERSION  = $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')

help:
	@echo "Pipemeeter build targets"
	@echo "  make binary       — release binary at target/release/pipemeeter"
	@echo "  make deb          — .deb package (requires cargo-deb)"
	@echo "  make aur-srcinfo  — regenerate packaging/aur/.SRCINFO from PKGBUILD"
	@echo "  make clean        — remove build artefacts"

binary:
	cargo build --release --features "$(FEATURES)"
	@echo "Binary: target/release/pipemeeter  (v$(VERSION))"

deb: binary
	@command -v cargo-deb >/dev/null 2>&1 || cargo install cargo-deb
	cargo deb --no-build --features "$(FEATURES)"
	@echo "Package: target/debian/pipemeeter_$(VERSION)_amd64.deb"

aur-srcinfo:
	@command -v makepkg >/dev/null 2>&1 || { echo "makepkg not found — run on Arch Linux"; exit 1; }
	cd packaging/aur && makepkg --printsrcinfo > .SRCINFO
	@echo "Updated packaging/aur/.SRCINFO"

clean:
	cargo clean
