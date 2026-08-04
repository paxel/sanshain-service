//! Chart rendering seam.
//!
//! The Helm chart gained `extraVolumes`/`extraVolumeMounts` so a CA certificate
//! directory can be mounted. Two properties matter and neither is visible from
//! Rust: existing installations must be unaffected when the new values are
//! unset, and the new values must actually reach the rendered pod spec.
//!
//! Skips when `helm` is not installed, so the suite still runs on a machine
//! without it rather than failing for the wrong reason.

use std::process::Command;

fn helm_available() -> bool {
    Command::new("helm")
        .arg("version")
        .arg("--short")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns the rendered manifests, or a message describing why rendering failed.
///
/// Fallibility is returned rather than asserted here so the panic happens inside
/// a `#[test]` function, where the project's clippy configuration permits it.
fn render(extra_args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("helm");
    cmd.arg("template").arg("t").arg("deploy/helm/sanshain");
    for arg in extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().map_err(|e| format!("running helm: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "helm template failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn default_render_mounts_only_the_data_volume() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    // The data volume is the only one an untouched installation gets.
    assert!(
        rendered.contains("name: data"),
        "the database volume must still be rendered"
    );
    assert_eq!(
        rendered.matches("persistentVolumeClaim:").count(),
        1,
        "exactly one claim by default"
    );
    assert!(
        !rendered.contains("EXTRA_CA_CERTS_DIR"),
        "the certificate option must be absent unless configured, so existing \
         installations are untouched"
    );
    assert!(
        !rendered.contains("ca-certs"),
        "no certificate volume by default"
    );
}

#[test]
fn configured_render_mounts_the_certificate_volume() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "config.extraCaCertsDir=/certs",
        "--set",
        "extraVolumes[0].name=ca-certs",
        "--set",
        "extraVolumes[0].secret.secretName=corp-ca",
        "--set",
        "extraVolumeMounts[0].name=ca-certs",
        "--set",
        "extraVolumeMounts[0].mountPath=/certs",
        "--set",
        "extraVolumeMounts[0].readOnly=true",
    ])
    .expect("chart renders with a certificate volume configured");

    assert!(
        rendered.contains(r#"EXTRA_CA_CERTS_DIR: "/certs""#),
        "the option must reach the ConfigMap, got:\n{rendered}"
    );
    assert!(
        rendered.contains("secretName: corp-ca"),
        "the certificate volume must be rendered"
    );
    assert!(
        rendered.contains("mountPath: /certs"),
        "the certificate mount must be rendered"
    );
    // The database volume must survive alongside it, not be displaced.
    assert!(
        rendered.contains("mountPath: /data"),
        "the data mount must still be present"
    );
}

#[test]
fn extra_volumes_render_without_persistence() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    // Persistence is disabled for an external PostgreSQL deployment, which used
    // to be the only thing emitting the volumes keys at all.
    let rendered = render(&[
        "--set",
        "persistence.enabled=false",
        "--set",
        "extraVolumes[0].name=ca-certs",
        "--set",
        "extraVolumes[0].secret.secretName=corp-ca",
        "--set",
        "extraVolumeMounts[0].name=ca-certs",
        "--set",
        "extraVolumeMounts[0].mountPath=/certs",
    ])
    .expect("chart renders with persistence disabled");

    assert!(
        rendered.contains("secretName: corp-ca"),
        "extra volumes must render even with persistence disabled, got:\n{rendered}"
    );
    assert!(
        rendered.contains("mountPath: /certs"),
        "extra mounts must render even with persistence disabled"
    );
    assert!(
        !rendered.contains("mountPath: /data"),
        "the data mount must be absent when persistence is disabled"
    );
}
