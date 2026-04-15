# Makefile — common development tasks for Scopeon contributors
#
# Usage:
#   make          — run all checks (fmt-check, clippy, test) — same as CI
#   make build    — dev build
#   make test     — run all tests
#   make install  — install scopeon binary to ~/.cargo/bin
#   make docs     — build and open rustdoc
#   make clean    — remove build artifacts
#   make check    — full pre-push gate (fmt + clippy + test + audit)
#   make ci-local — run EVERY CI job locally (closest to the real CI pipeline)

.PHONY: all build release test clippy fmt fmt-check check ci-local audit deny docs install clean

CARGO ?= cargo

# Default: run the same checks that CI requires (fmt-check + clippy + test).
all: fmt-check clippy test

# ── Build ───────────────────────────────────────────────────────────────────

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --release --workspace

install:
	$(CARGO) install --path . --locked

# ── Tests ───────────────────────────────────────────────────────────────────

test:
	$(CARGO) test --workspace

test-verbose:
	$(CARGO) test --workspace -- --nocapture

# ── Lint & format ──────────────────────────────────────────────────────────

clippy:
	$(CARGO) clippy --workspace -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

# ── Security ────────────────────────────────────────────────────────────────

audit:
	$(CARGO) audit

deny:
	$(CARGO) deny check

# ── Documentation ──────────────────────────────────────────────────────────

docs:
	$(CARGO) doc --workspace --no-deps --open

docs-check:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps

# ── Full pre-push check (run before opening a PR) ───────────────────────────
# Mirrors what CI requires + audit. Docs not included because they require
# a clean build which is slow locally — run `make docs-check` separately.

check: fmt-check clippy test audit
	@echo ""
	@echo "✅ All checks passed. Safe to push."

# ── Full CI equivalent (mirrors every CI job) ───────────────────────────────

ci-local: fmt-check clippy test docs-check audit
	@echo ""
	@echo "✅ Full CI-equivalent check passed."

# ── Coverage ──────────────────────────────────────────────────────────────
# Requires: cargo install cargo-llvm-cov

coverage:
	$(CARGO) llvm-cov --workspace --open

coverage-summary:
	$(CARGO) llvm-cov --workspace --summary-only

# ── Cleanup ─────────────────────────────────────────────────────────────────

clean:
	$(CARGO) clean
