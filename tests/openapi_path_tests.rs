use sanshain_service::openapi::normalize_path;

// 1. collapses multiple slashes
#[test]
fn normalize_collapses_slashes() {
    assert_eq!(normalize_path("//api///v1//users"), "/api/v1/users");
}

// 2. trims trailing slash
#[test]
fn normalize_trims_trailing_slash() {
    assert_eq!(normalize_path("/api/users/"), "/api/users");
}

// 3. keeps single root slash
#[test]
fn normalize_root_slash_kept() {
    assert_eq!(normalize_path("/"), "/");
}

// 4. replaces variables with {}
#[test]
fn normalize_replaces_variables() {
    assert_eq!(normalize_path("/users/{id}/orders/{oid:[0-9]+}"), "/users/{}/orders/{}");
}

// 5. trims whitespace
#[test]
fn normalize_trims_whitespace() {
    assert_eq!(normalize_path("  /api/users  "), "/api/users");
}

// 6. idempotent on already-normalized
#[test]
fn normalize_idempotent() {
    let p = "/a/{}/b";
    assert_eq!(normalize_path(p), p);
}
