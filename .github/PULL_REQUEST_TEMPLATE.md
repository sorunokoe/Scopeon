## Description

<!-- What does this PR change? Why? Link the related issue if one exists. -->

Closes #<!-- issue number -->

## Type of change

- [ ] Bug fix
- [ ] New feature / provider support
- [ ] Refactoring (no behaviour change)
- [ ] Documentation update
- [ ] Performance improvement
- [ ] CI / tooling change

## CI checks

All of the following jobs must be green (shown as **CI pass** ✅) before merging:

| Job | What it checks |
|-----|----------------|
| **Format check** | `cargo fmt --all -- --check` |
| **Clippy** | `cargo clippy --workspace -- -D warnings` |
| **Test (ubuntu)** | `cargo test --workspace --locked` |
| **Test (macos)** | `cargo test --workspace --locked` |
| **Test (windows)** | `cargo test --workspace --locked` |
| **MSRV (1.88)** | Builds on the minimum supported Rust version |
| **Release build** | `cargo build --release --locked` |
| **Docs** | `cargo doc --no-deps` — no broken links or warnings |

Advisory only (do not block merge, but watch for regressions):

| Job | What it checks |
|-----|----------------|
| **Security audit** | `cargo audit` — no known vulnerabilities |
| **Dependency check** | `cargo-deny` — license allowlist + ban list |
| **Coverage** | `cargo-llvm-cov` — line coverage summary |

Run `make check` locally to verify fmt + clippy + tests + audit before pushing.

## Checklist

- [ ] `make check` passes locally (`cargo fmt` + `cargo clippy` + `cargo test` + `cargo audit`)
- [ ] Added/updated tests for the changed logic
- [ ] Updated relevant doc comments (`///` / `//!`)
- [ ] CHANGELOG entry added under `[Unreleased]`
- [ ] No `unwrap()` in library code (use `?` / `Result`)
- [ ] No new `unsafe` code without a `// SAFETY:` comment
- [ ] Manually tested with a real provider session (if applicable)
