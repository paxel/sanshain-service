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

// --- Backup CronJobs (#50) ---

#[test]
fn no_backup_cronjob_unless_enabled() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    assert!(
        !rendered.contains("kind: CronJob"),
        "an untouched installation gets no backup CronJob"
    );
    assert!(
        !rendered.contains("-backup"),
        "no backup claim either, got:\n{rendered}"
    );
}

#[test]
fn sqlite_backup_cronjob_curls_the_snapshot_with_the_token_secret() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "backup.enabled=true",
        "--set",
        "backup.schedule=30 3 * * *",
        "--set",
        "backup.retain=14",
        "--set",
        "backup.tokenSecret.name=sanshain-backup-token",
        "--set",
        "backup.tokenSecret.key=apiToken",
    ])
    .expect("chart renders the SQLite backup CronJob");

    assert!(rendered.contains("kind: CronJob"), "got:\n{rendered}");
    assert!(
        rendered.contains(r#"schedule: "30 3 * * *""#),
        "the schedule comes from values, got:\n{rendered}"
    );
    assert!(
        rendered.contains("/admin/backup/sqlite"),
        "the job fetches the snapshot route"
    );
    assert!(
        !rendered.contains("pg_dump"),
        "the SQLite variant does not run pg_dump"
    );
    assert!(
        rendered.contains("name: sanshain-backup-token") && rendered.contains("key: apiToken"),
        "the bearer token is read from the referenced secret, got:\n{rendered}"
    );
    assert!(
        rendered.contains("http://t-sanshain:80"),
        "the job talks to the service's cluster DNS name, got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"value: "14""#),
        "the retention count reaches the job"
    );
    // Without an existing claim the chart creates one and mounts it.
    assert!(
        rendered.contains("claimName: t-sanshain-backup"),
        "the job mounts the chart's backup claim"
    );
    assert_eq!(
        rendered.matches("kind: PersistentVolumeClaim").count(),
        2,
        "the backup PersistentVolumeClaim is rendered beside the data one"
    );
}

#[test]
fn backup_cronjob_uses_an_existing_claim_instead_of_creating_one() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "backup.enabled=true",
        "--set",
        "backup.existingClaim=nightly-backups",
        "--set",
        "backup.tokenSecret.name=sanshain-backup-token",
    ])
    .expect("chart renders with an existing backup claim");

    assert!(
        rendered.contains("claimName: nightly-backups"),
        "the job mounts the operator's claim, got:\n{rendered}"
    );
    assert_eq!(
        rendered.matches("kind: PersistentVolumeClaim").count(),
        1,
        "no backup claim is created when one is given"
    );
}

#[test]
fn sqlite_backup_requires_the_token_secret() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let err = render(&["--set", "backup.enabled=true"])
        .expect_err("the SQLite backup cannot authenticate without a token secret");
    assert!(
        err.contains("backup.tokenSecret.name"),
        "the failure names the missing value, got:\n{err}"
    );
}

#[test]
fn postgres_backup_cronjob_runs_pg_dump_against_the_database_url() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    // Explicit engine.
    let rendered = render(&[
        "--set",
        "backup.enabled=true",
        "--set",
        "backup.engine=postgres",
        "--set",
        "backup.schedule=0 4 * * *",
    ])
    .expect("chart renders the PostgreSQL backup CronJob");

    assert!(rendered.contains("kind: CronJob"), "got:\n{rendered}");
    assert!(
        rendered.contains("pg_dump"),
        "the PostgreSQL variant runs pg_dump"
    );
    assert!(
        !rendered.contains("/admin/backup/sqlite"),
        "the PostgreSQL variant does not call the snapshot route"
    );
    assert!(
        rendered.contains(r#"schedule: "0 4 * * *""#),
        "the schedule comes from values"
    );
    assert!(
        rendered.contains("name: t-sanshain-secrets") && rendered.contains("key: DATABASE_URL"),
        "DATABASE_URL is read from the chart's existing secret, got:\n{rendered}"
    );

    // Derived from the database URL when the engine is not set.
    let derived = render(&[
        "--set",
        "backup.enabled=true",
        "--set",
        "secrets.databaseUrl=postgres://u:p@db:5432/sanshain",
    ])
    .expect("chart derives the engine from the database URL");
    assert!(
        derived.contains("pg_dump") && !derived.contains("/admin/backup/sqlite"),
        "a postgres:// URL selects the pg_dump variant, got:\n{derived}"
    );
}
