---
name: secure-csrf
description: Enforces secure CSRF and authentication token handling, forbidding hardcoded test bypasses or backdoors in production code.
---

# Secure CSRF & Auth Token Handling

## Purpose
Prevent security breaches caused by test scaffolding leaking into production code.
Security checks (CSRF tokens, auth tokens, API keys, sessions) MUST be validated through the
real validation path and MUST NOT be weakened, short-circuited, or bypassed to make tests pass.

This skill exists because a hardcoded `if token == "test-csrf-token" { allow }` bypass was once
shipped in production middleware, fully disabling CSRF for any caller who knew the magic string.

## Trigger
React when working on or reviewing:
- CSRF middleware / validation
- Authentication, authorization, session, or API-token logic
- Any code comparing a request value against a secret or token
- Any test that needs to pass through a security gate (CSRF/auth)

## The Rule (Non-Negotiable)
1. **No magic strings**: Production code MUST NOT contain hardcoded token/secret literals
   (e.g. `"test-csrf-token"`, `"dev-key"`, `"admin"`, `"bypass"`) used to grant access.
2. **No conditional backdoors**: No branch may skip validation based on a well-known constant,
   header presence alone, env flag, or "test mode" baked into the release binary.
3. **Single validation path**: Tests must exercise the SAME validation logic as production.
   To pass a security gate in tests, seed a real, valid credential into the real store
   (e.g. insert a CSRF token with a future expiry into `csrf_tokens`), never special-case it in code.
4. **Test-only code must be compiled out**: If a shortcut is unavoidable, gate it behind
   `#[cfg(test)]` so it can never exist in a release build — never plain runtime `if`.
5. **Constant-time where it matters**: Compare high-value secrets without early-exit string equality
   when feasible; rely on high-entropy random tokens.

## Guidelines
- **Fail closed**: Default to denying access; grant only after positive validation.
- **One path**: Production and tests must run identical validation logic.
- **No release backdoors**: Any test affordance must be compiled out via `#[cfg(test)]`.
- **High entropy**: Generate tokens from a CSPRNG and store/compare them safely.

## Procedures

### When Implementing a Security Check
1. Validate the credential against its real store/expiry/signature.
2. Reject by default (`FORBIDDEN`/`UNAUTHORIZED`); allow only on successful validation.
3. Do not add any literal-string comparison as an allow condition.

### When a Test Needs to Pass the Gate
1. Generate or register a genuine credential in the real backing store used by production.
   - CSRF example: insert the token into `state.csrf_tokens` with `Utc::now() + Duration::hours(1)`.
   - Auth example: create a real session/API token via the normal service functions.
2. Send it via the normal header the production path reads.
3. Never modify production code to recognize a test value.

### Review / Audit Checklist
- [ ] No hardcoded token/secret literals in non-`#[cfg(test)]` code.
- [ ] No `if <value> == "<constant>" { return Ok/allow }` in security middleware.
- [ ] Validation is reject-by-default.
- [ ] Tests seed real credentials instead of triggering a bypass.
- [ ] Any unavoidable shortcut is behind `#[cfg(test)]`.
- [ ] `grep -rn "test-csrf\|bypass\|backdoor\|dev-key" src/` returns nothing actionable.

## Example

Forbidden (production backdoor):
```rust
if let Some(token) = csrf_header {
    if token == "test-csrf-token" {        // ❌ ships a backdoor
        return Ok(next.run(req).await);
    }
    // ...
}
```

Correct (real validation; tests seed a real token):
```rust
if let Some(token) = csrf_header {
    let tokens = state.csrf_tokens.read().await;
    if let Some(expiry) = tokens.get(token) && *expiry > chrono::Utc::now() {
        return Ok(next.run(req).await);    // ✅ same path for prod and tests
    }
}
Err(StatusCode::FORBIDDEN)
```
