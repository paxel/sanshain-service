use sanshain_service::domain::models::*;
use std::str::FromStr;

// 1-3. ApiType::as_str for each variant
#[test]
fn api_type_as_str_values() {
    assert_eq!(ApiType::OpenApi.as_str(), "openapi");
    assert_eq!(ApiType::AsyncApi.as_str(), "asyncapi");
    assert_eq!(ApiType::Proto.as_str(), "proto");
}

// 4-8. ApiType::from_str mappings and alias support
#[test]
fn api_type_from_str_mappings() {
    assert!(matches!(
        ApiType::from_str("openapi").unwrap(),
        ApiType::OpenApi
    ));
    assert!(matches!(
        ApiType::from_str("rest").unwrap(),
        ApiType::OpenApi
    ));
    assert!(matches!(
        ApiType::from_str("asyncapi").unwrap(),
        ApiType::AsyncApi
    ));
    assert!(matches!(
        ApiType::from_str("kafka").unwrap(),
        ApiType::AsyncApi
    ));
    assert!(matches!(ApiType::from_str("grpc").unwrap(), ApiType::Proto));
}

// 9. ApiType::from_str unknown returns Err
#[test]
fn api_type_from_str_unknown_err() {
    assert!(ApiType::from_str("soap").is_err());
}

// 10-12. AuthMode from_str and as_str
#[test]
fn auth_mode_from_and_as_str() {
    assert_eq!(AuthMode::from_str("dev").unwrap().as_str(), "dev");
    assert_eq!(AuthMode::from_str("local").unwrap().as_str(), "local");
    assert_eq!(AuthMode::from_str("ldap").unwrap().as_str(), "ldap");
}

// 13. AuthMode invalid
#[test]
fn auth_mode_invalid_err() {
    assert!(AuthMode::from_str("sso").is_err());
}

// 14. ProvideChanges default is zeros
#[test]
fn provide_changes_default_zeros() {
    let ch = ProvideChanges::default();
    assert_eq!(ch.inserts, 0);
    assert_eq!(ch.updates, 0);
    assert_eq!(ch.deletes, 0);
}
