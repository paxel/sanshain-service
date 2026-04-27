# Junie Skill: Quality Check

Run the full quality suite for the Sanshain Service project.

## Steps

### 1. Rust Formatting
```bash
cargo fmt --check
```
**Expected**: Exit 0, no output. If diffs appear, run `cargo fmt` to fix.

### 2. Rust Linting
```bash
cargo clippy -- -D warnings
```
**Expected**: Exit 0, zero warnings. Never use `#[allow(...)]` to suppress.

### 3. Rust Tests
```bash
cargo test --lib
```
**Expected**: All tests pass. Currently 59 lib tests.

### 4. Security Audit
```bash
cargo audit
```
**Expected**: Exit 0 or only documented ignores.

### 5. JavaScript Linting
```bash
npx eslint static/js/
```
**Expected**: Exit 0, no errors. Cross-file globals are declared in `eslint.config.js`.

### 6. JavaScript Formatting
```bash
npx prettier --check static/js/
```
**Expected**: Exit 0. Fix with `npx prettier --write static/js/`.

## Thresholds

| Check | Threshold |
|-------|-----------|
| Clippy warnings | 0 |
| Lib test failures | 0 |
| ESLint errors | 0 |
| Prettier violations | 0 |
| Production `.unwrap()` | 0 |

## Troubleshooting

- **Clippy fails after dependency update**: Run `cargo update` then retry.
- **ESLint "not defined" error**: Add the global to `eslint.config.js` under `languageOptions.globals`.
- **Prettier diffs**: Run `npx prettier --write static/js/` and review changes.
- **Node.js not available**: Install via `nvm install 20` (nvm is configured in the project).
- **cargo-audit fails on known advisory**: Document the ignore in `ai/quality-hardening.md` with justification.
