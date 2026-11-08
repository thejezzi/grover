# PREFIX defaults to a user install; `make install PREFIX=/usr/local` needs sudo.
PREFIX ?= $(HOME)/.local
BINDIR := $(DESTDIR)$(PREFIX)/bin
BIN := target/release/grover

.PHONY: build install uninstall check test

build:
	cargo build --release

install: build
	install -D -m 0755 $(BIN) $(BINDIR)/grover

uninstall:
	rm -f $(BINDIR)/grover

check: test
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings

test:
	cargo test
