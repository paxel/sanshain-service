//! Additional CA certificate trust for outbound TLS.
//!
//! Operators running Sanshain behind a corporate CA mount a directory of PEM
//! certificates and point [`EXTRA_CA_CERTS_DIR_ENV`] at it. Those certificates
//! are *added* to the platform trust store, never substituted for it, so public
//! authorities keep working alongside an internal one.
//!
//! The trust store is built here and handed to the LDAP client for **every**
//! connection, whether or not extra certificates were configured. Left to its
//! own devices ldap3 collapses to an *empty* root store if reading the platform
//! certificates produces any error, decides that once for the process lifetime,
//! and reports nothing — a service that trusts nothing and says so nowhere.
//! Supplying the configuration also bypasses ldap3's `ClientConfig::builder()`,
//! which panics outright here because two crypto providers are linked in (`ring`
//! via ldap3, `aws-lc-rs` via metrics-exporter-prometheus → hyper-rustls) and
//! rustls refuses to guess between them.

use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Names a directory of PEM certificates to trust in addition to the platform's.
pub const EXTRA_CA_CERTS_DIR_ENV: &str = "EXTRA_CA_CERTS_DIR";

/// Extensions treated as certificate files.
///
/// A mounted volume routinely carries incidental entries (`..data`,
/// `..2026_01_01`, README files); ignoring those by extension keeps them from
/// failing startup while a genuinely unreadable certificate still does. `crt`
/// and `cer` are accepted alongside `pem` because a Kubernetes Secret is
/// conventionally keyed `ca.crt`, and silently trusting nothing because of a
/// file extension is exactly the quiet failure this module exists to prevent.
const CERT_EXTENSIONS: [&str; 3] = ["pem", "crt", "cer"];

#[derive(Debug, thiserror::Error)]
pub enum TrustStoreError {
    #[error("CA certificate directory '{path}' does not exist")]
    DirectoryMissing { path: String },

    #[error("Could not read CA certificate directory '{path}': {source}")]
    DirectoryUnreadable {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not read CA certificate file '{path}': {message}")]
    MalformedCertificate { path: String, message: String },

    #[error("Could not build the TLS client configuration: {message}")]
    ClientConfig { message: String },
}

/// Certificates read from an extra CA directory, with everything a startup log
/// line needs. Certificate contents are never logged — only these counts and
/// the directory they came from.
#[derive(Debug)]
pub struct ExtraCaCerts {
    pub certs: Vec<CertificateDer<'static>>,
    /// Number of certificate files that were read, which may differ from the
    /// number of certificates when a file holds a chain.
    pub files_read: usize,
    /// The directory these were read from.
    pub directory: PathBuf,
}

/// Process-wide LDAP client configuration, installed once at startup.
static LDAP_CLIENT_CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();

/// Read every certificate file in `dir` (see [`CERT_EXTENSIONS`]).
///
/// Fails when the directory is absent or unreadable, and when a certificate
/// file cannot be parsed or holds no certificate at all — a file that parses to
/// nothing means the operator mounted something other than what they intended,
/// which is worth refusing to start over.
pub fn load_extra_ca_certs(dir: &Path) -> Result<ExtraCaCerts, TrustStoreError> {
    if !dir.exists() {
        return Err(TrustStoreError::DirectoryMissing {
            path: dir.display().to_string(),
        });
    }

    let entries =
        std::fs::read_dir(dir).map_err(|source| TrustStoreError::DirectoryUnreadable {
            path: dir.display().to_string(),
            source,
        })?;

    // Sorted so that startup logging and any error reported are deterministic
    // rather than dependent on directory iteration order.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| TrustStoreError::DirectoryUnreadable {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_cert = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| CERT_EXTENSIONS.iter().any(|c| e.eq_ignore_ascii_case(c)));
        if is_cert {
            paths.push(path);
        }
    }
    paths.sort();

    let mut certs = Vec::new();
    let mut files_read = 0usize;

    for path in &paths {
        let iter = CertificateDer::pem_file_iter(path).map_err(|e| {
            TrustStoreError::MalformedCertificate {
                path: path.display().to_string(),
                message: e.to_string(),
            }
        })?;

        let mut found_in_file = 0usize;
        for cert in iter {
            let cert = cert.map_err(|e| TrustStoreError::MalformedCertificate {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
            certs.push(cert);
            found_in_file += 1;
        }

        if found_in_file == 0 {
            return Err(TrustStoreError::MalformedCertificate {
                path: path.display().to_string(),
                message: "file contains no certificates".to_string(),
            });
        }

        files_read += 1;
    }

    Ok(ExtraCaCerts {
        certs,
        files_read,
        directory: dir.to_path_buf(),
    })
}

/// The platform trust store.
///
/// Unlike the LDAP client's loader, a partial failure yields the certificates
/// that *did* load rather than an empty store, and the failure is logged.
pub fn native_root_store() -> RootCertStore {
    let mut store = RootCertStore::empty();
    let result = rustls_native_certs::load_native_certs();

    for error in &result.errors {
        tracing::warn!("Could not load a platform CA certificate: {}", error);
    }

    for cert in result.certs {
        if let Err(e) = store.add(cert) {
            tracing::warn!("Rejected a platform CA certificate: {}", e);
        }
    }

    store
}

/// Platform trust store plus `extra`.
pub fn build_root_store(extra: &[CertificateDer<'static>]) -> RootCertStore {
    let mut store = native_root_store();
    for cert in extra {
        if let Err(e) = store.add(cert.clone()) {
            tracing::warn!("Rejected an additional CA certificate: {}", e);
        }
    }
    store
}

/// Read the extra CA directory named by the environment, if any.
///
/// `Ok(None)` when the variable is unset or empty.
fn read_extra_from_env() -> Result<Option<ExtraCaCerts>, TrustStoreError> {
    let Ok(dir) = std::env::var(EXTRA_CA_CERTS_DIR_ENV) else {
        return Ok(None);
    };
    if dir.is_empty() {
        return Ok(None);
    }
    load_extra_ca_certs(Path::new(&dir)).map(Some)
}

/// Build the LDAP client configuration and install it for the process.
///
/// Runs whether or not extra certificates are configured: the configuration is
/// what keeps ldap3 off its own `ClientConfig::builder()` (which panics here)
/// and off its empty-root-store fallback. Any failure is returned so the caller
/// can refuse to start rather than run with a trust store that silently lacks
/// what the operator mounted.
///
/// Must be called before the first LDAP connection. It logs what it installed;
/// certificate contents are never logged.
pub fn init_from_env() -> Result<(), TrustStoreError> {
    let loaded = read_extra_from_env()?;
    let extra: &[CertificateDer<'static>] = loaded.as_ref().map_or(&[], |l| &l.certs);
    let config = build_client_config(build_root_store(extra))?;

    if LDAP_CLIENT_CONFIG.set(config).is_err() {
        // Something already initialised it, so the certificates just read are
        // not the ones in use. Say so rather than reporting a successful load.
        tracing::warn!(
            "LDAP client configuration was already initialised; certificates read now are NOT in use"
        );
        return Ok(());
    }

    match loaded {
        Some(loaded) => tracing::info!(
            "Trusting {} additional CA certificate(s) from {} file(s) in {}",
            loaded.certs.len(),
            loaded.files_read,
            loaded.directory.display()
        ),
        None => tracing::debug!(
            "No additional CA certificates configured ({} unset); using platform trust store",
            EXTRA_CA_CERTS_DIR_ENV
        ),
    }

    Ok(())
}

/// Build the client configuration for `store`.
///
/// The provider is named explicitly. Two are compiled in — `ring` via ldap3 and
/// `aws-lc-rs` via metrics-exporter-prometheus → hyper-rustls — and with both
/// present rustls refuses to choose: the convenient `ClientConfig::builder()`
/// *panics* instead of returning an error. `ring` is chosen to match the
/// provider ldap3 is built against.
fn build_client_config(store: RootCertStore) -> Result<Arc<rustls::ClientConfig>, TrustStoreError> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| TrustStoreError::ClientConfig {
        message: e.to_string(),
    })?
    .with_root_certificates(store)
    .with_no_client_auth();

    Ok(Arc::new(config))
}

/// The client configuration to use for every LDAP connection.
///
/// Falls back to a platform-only trust store if [`init_from_env`] has not run,
/// so a caller can never end up handing ldap3 no configuration at all — which
/// would put it back on the `ClientConfig::builder()` path that panics here.
/// In the service `init_from_env` runs at startup, before the listener binds,
/// so it is always the one that populates this.
pub fn ldap_client_config() -> Option<Arc<rustls::ClientConfig>> {
    if let Some(config) = LDAP_CLIENT_CONFIG.get() {
        return Some(config.clone());
    }
    match build_client_config(native_root_store()) {
        Ok(config) => Some(LDAP_CLIENT_CONFIG.get_or_init(|| config).clone()),
        Err(e) => {
            tracing::error!("Could not build the LDAP TLS client configuration: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed, self-signed test CAs. Generated once and checked in — never
    /// generated at test time, so results do not vary between runs.
    const CA_A: &str = "\
-----BEGIN CERTIFICATE-----
MIIBkjCCATegAwIBAgIUIKbhyedlNWVWs/f9zuoENhEZjY0wCgYIKoZIzj0EAwIw
HTEbMBkGA1UEAwwSU2Fuc2hhaW4gVGVzdCBDQSBhMCAXDTI2MDcyODE5MjExOVoY
DzIxMjYwNzA0MTkyMTE5WjAdMRswGQYDVQQDDBJTYW5zaGFpbiBUZXN0IENBIGEw
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAATfVKx9siXOzsMwYPJxvLZnGkqGdjMU
kieKmqc7jFNZp267QnewK43mLZfz6hVDXnyult8yT/6Ia0/gBRZuyg5Xo1MwUTAd
BgNVHQ4EFgQUoY6waVLqw92r+ZdSgCb28KK15u0wHwYDVR0jBBgwFoAUoY6waVLq
w92r+ZdSgCb28KK15u0wDwYDVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNJADBG
AiEArsQhQ/IagvuxzG8yR278JRqj4s/bCx2JPoL20k4ytD8CIQCt6h7phmF+FBy0
j3N86gZjxhc6j1rhIEZ/TxIVFryO7g==
-----END CERTIFICATE-----
";

    const CA_B: &str = "\
-----BEGIN CERTIFICATE-----
MIIBkDCCATegAwIBAgIUZFD/s+iysWjyzuNxYArP/pDylNUwCgYIKoZIzj0EAwIw
HTEbMBkGA1UEAwwSU2Fuc2hhaW4gVGVzdCBDQSBiMCAXDTI2MDcyODE5MjExOVoY
DzIxMjYwNzA0MTkyMTE5WjAdMRswGQYDVQQDDBJTYW5zaGFpbiBUZXN0IENBIGIw
WTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAR1E/uQLCQCy7le9Gov1M31RmzQRP/B
4xM++Fk6w4WaqTIgG9sNzwaJXVLTYj5cH/yB+imMzw1gslAcs/t7LNDAo1MwUTAd
BgNVHQ4EFgQUF90VSGDygA84oHEDDVkTBZR0DuUwHwYDVR0jBBgwFoAUF90VSGDy
gA84oHEDDVkTBZR0DuUwDwYDVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNHADBE
AiBcvV/BtfsFGa/mXZVz0gP/MBsPJFNgQ2rSk1oflg041gIgajss/jzDbsiMEW0r
ia0rU+VKt226rPiZA1b79i4t9mQ=
-----END CERTIFICATE-----
";

    /// A deterministically named scratch directory per test, recreated on entry
    /// so a previous run's leftovers cannot influence the result.
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sanshain_tls_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write fixture");
    }

    #[test]
    fn empty_directory_loads_nothing() {
        let dir = scratch_dir("empty");

        let loaded = load_extra_ca_certs(&dir).expect("empty directory is not an error");

        assert_eq!(loaded.certs.len(), 0);
        assert_eq!(loaded.files_read, 0);
    }

    #[test]
    fn single_certificate_is_loaded() {
        let dir = scratch_dir("single");
        write(&dir, "corp-ca.pem", CA_A);

        let loaded = load_extra_ca_certs(&dir).expect("valid certificate loads");

        assert_eq!(loaded.certs.len(), 1);
        assert_eq!(loaded.files_read, 1);
    }

    #[test]
    fn several_certificates_are_all_loaded() {
        let dir = scratch_dir("several");
        write(&dir, "ca-a.pem", CA_A);
        write(&dir, "ca-b.pem", CA_B);

        let loaded = load_extra_ca_certs(&dir).expect("valid certificates load");

        assert_eq!(loaded.certs.len(), 2);
        assert_eq!(loaded.files_read, 2);
    }

    #[test]
    fn certificate_chain_in_one_file_is_loaded() {
        let dir = scratch_dir("chain");
        write(&dir, "chain.pem", &format!("{CA_A}{CA_B}"));

        let loaded = load_extra_ca_certs(&dir).expect("a chain loads");

        assert_eq!(loaded.certs.len(), 2, "both certificates in the chain");
        assert_eq!(loaded.files_read, 1, "read from a single file");
    }

    #[test]
    fn malformed_certificate_fails_and_names_the_file() {
        let dir = scratch_dir("malformed");
        write(&dir, "good.pem", CA_A);
        write(
            &dir,
            "broken.pem",
            "-----BEGIN CERTIFICATE-----\nnot base64 at all!!\n-----END CERTIFICATE-----\n",
        );

        let err = load_extra_ca_certs(&dir).expect_err("a malformed certificate must fail");

        let message = err.to_string();
        assert!(
            message.contains("broken.pem"),
            "error must name the offending file, got: {message}"
        );
    }

    #[test]
    fn pem_file_without_a_certificate_fails() {
        let dir = scratch_dir("no_cert");
        write(
            &dir,
            "just-a-key.pem",
            "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIA==\n-----END PRIVATE KEY-----\n",
        );

        let err = load_extra_ca_certs(&dir).expect_err("a file with no certificate must fail");

        let message = err.to_string();
        assert!(
            message.contains("just-a-key.pem"),
            "error must name the offending file, got: {message}"
        );
        assert!(
            message.contains("no certificates"),
            "error must say why, got: {message}"
        );
    }

    #[test]
    fn missing_directory_fails() {
        let dir = std::env::temp_dir().join("sanshain_tls_test_definitely_absent");
        let _ = std::fs::remove_dir_all(&dir);

        let err = load_extra_ca_certs(&dir).expect_err("a missing directory must fail");

        assert!(matches!(err, TrustStoreError::DirectoryMissing { .. }));
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn unrelated_files_are_ignored() {
        let dir = scratch_dir("unrelated");
        write(&dir, "ca-a.pem", CA_A);
        write(&dir, "README.txt", "certificates live here");
        write(&dir, "..data", "kubernetes projection artefact");
        write(&dir, "notes.md", "# not a certificate");

        let loaded = load_extra_ca_certs(&dir).expect("unrelated files are skipped");

        assert_eq!(loaded.certs.len(), 1);
        assert_eq!(loaded.files_read, 1);
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let dir = scratch_dir("case");
        write(&dir, "corp-ca.PEM", CA_A);

        let loaded = load_extra_ca_certs(&dir).expect("uppercase extension is still a PEM file");

        assert_eq!(loaded.certs.len(), 1);
    }

    /// A Kubernetes Secret is conventionally keyed `ca.crt`, which projects to a
    /// file of that name. Ignoring it would trust nothing while starting
    /// successfully — the quiet failure this module exists to prevent.
    #[test]
    fn kubernetes_ca_crt_convention_is_loaded() {
        let dir = scratch_dir("ca_crt");
        write(&dir, "ca.crt", CA_A);
        write(&dir, "intermediate.cer", CA_B);

        let loaded = load_extra_ca_certs(&dir).expect("crt and cer are certificate files");

        assert_eq!(loaded.certs.len(), 2);
        assert_eq!(loaded.files_read, 2);
    }

    #[test]
    fn loaded_certificates_record_their_directory() {
        let dir = scratch_dir("directory");
        write(&dir, "ca-a.pem", CA_A);

        let loaded = load_extra_ca_certs(&dir).expect("fixtures load");

        assert_eq!(loaded.directory, dir);
    }

    #[test]
    fn extra_certificates_are_added_to_the_platform_store() {
        let platform_only = native_root_store().len();

        let certs = {
            let dir = scratch_dir("added");
            write(&dir, "ca-a.pem", CA_A);
            write(&dir, "ca-b.pem", CA_B);
            load_extra_ca_certs(&dir).expect("fixtures load").certs
        };

        let combined = build_root_store(&certs).len();

        assert_eq!(combined, platform_only + 2, "both extra CAs were added");

        // Additivity above is the real invariant. Retention only says something
        // where there is something to retain, so assert it explicitly rather
        // than let the count silently prove nothing on a bare image with no
        // platform certificates at all.
        if platform_only > 0 {
            assert!(
                combined > platform_only,
                "platform certificates are kept alongside the extras, not replaced by them"
            );
        }
    }

    #[test]
    fn build_root_store_without_extras_matches_the_platform_store() {
        assert_eq!(build_root_store(&[]).len(), native_root_store().len());
    }

    /// Two crypto providers are compiled in: `ring` arrives via ldap3, and
    /// `aws-lc-rs` via metrics-exporter-prometheus → hyper-rustls. Feature
    /// unification means one rustls build carries both, and rustls then refuses
    /// to guess which to use — `ClientConfig::builder()` panics rather than
    /// returning an error. Building the config must therefore name its provider
    /// explicitly, and this test is what proves it still does.
    #[test]
    fn client_config_builds_with_both_crypto_providers_compiled_in() {
        let store = build_root_store(&[]);

        let config = build_client_config(store).expect("the client configuration must build");

        assert!(
            !config.crypto_provider().cipher_suites.is_empty(),
            "the configured provider must offer cipher suites"
        );
    }

    /// ldap3 is only kept off its panicking `ClientConfig::builder()` path if
    /// this always yields a configuration — including when `init_from_env` has
    /// never run, which is the case in this test binary.
    #[test]
    fn ldap_client_config_is_available_without_explicit_initialisation() {
        let config = ldap_client_config().expect("a configuration must always be available");

        assert!(
            !config.crypto_provider().cipher_suites.is_empty(),
            "the fallback configuration must be usable, not a husk"
        );

        // Same instance on a second call, so every connection shares one store
        // rather than rebuilding it per request.
        let again = ldap_client_config().expect("still available");
        assert!(
            Arc::ptr_eq(&config, &again),
            "the configuration is built once and reused"
        );
    }
}
