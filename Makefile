# NOTCH-ZAP-INATOR
#
#   make bundle      build dist/NotchZapInator.app (release, aarch64, ad-hoc signed)
#   make install     copy it to ~/Applications, start the LaunchAgent, mute the macOS charging chime
#   make uninstall   stop the LaunchAgent, remove both, restore the chime
#   make demo        play ZAP once from the release build
#   make check       build, clippy and tests (zero warnings expected)
#   make release     publish: zapinator.kunalbagaria.com + the Homebrew cask
#                    (bump `version` in Cargo.toml first)
#
# Pass CARGO_FLAGS=--no-default-features to build without sound, and
# MUTE_CHIME=0 to keep macOS's own "plugged in" chime.

LABEL      := com.kunalbagaria.zapinator
APP_NAME   := NotchZapInator.app
APP        := dist/$(APP_NAME)
INSTALL_TO := $(HOME)/Applications/$(APP_NAME)
AGENT      := $(HOME)/Library/LaunchAgents/$(LABEL).plist
DOMAIN     := gui/$(shell id -u)
CARGO_FLAGS ?=
MUTE_CHIME  ?= 1

.PHONY: bundle install uninstall release release-dry-run demo check logs clean chime-mute chime-restore

bundle:
	scripts/bundle.sh $(CARGO_FLAGS)

install: bundle
	/bin/bash "$(APP)/Contents/Resources/agent.sh" install --into "$(HOME)/Applications" $(if $(filter 0,$(MUTE_CHIME)),--keep-chime)

uninstall:
	@if [ -f "$(INSTALL_TO)/Contents/Resources/agent.sh" ]; then \
		/bin/bash "$(INSTALL_TO)/Contents/Resources/agent.sh" uninstall; \
	else \
		launchctl bootout $(DOMAIN)/$(LABEL) 2>/dev/null || true; \
		rm -f "$(AGENT)"; \
		scripts/chime.sh restore; \
	fi
	rm -rf "$(INSTALL_TO)"
	@echo "uninstalled"

release:
	scripts/release.sh

release-dry-run:
	scripts/release.sh --dry-run

demo:
	cargo run --release $(CARGO_FLAGS) -- --demo

check:
	cargo build $(CARGO_FLAGS)
	cargo clippy --all-targets $(CARGO_FLAGS) -- -D warnings
	cargo test $(CARGO_FLAGS)

chime-mute:
	scripts/chime.sh mute

chime-restore:
	scripts/chime.sh restore

logs:
	tail -f "$(HOME)/Library/Logs/NotchZapInator.log"

clean:
	cargo clean
	rm -rf dist
