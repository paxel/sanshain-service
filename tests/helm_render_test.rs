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
fn backup_retain_below_one_fails_the_render() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let err = render(&[
        "--set",
        "backup.enabled=true",
        "--set",
        "backup.engine=postgres",
        "--set",
        "backup.retain=0",
    ])
    .expect_err("retain 0 would delete the backup just produced");
    assert!(
        err.contains("backup.retain"),
        "the failure names the value, got:\n{err}"
    );
}

#[test]
fn backup_cronjobs_remove_stale_partial_files_first() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    for engine in ["sqlite", "postgres"] {
        let rendered = render(&[
            "--set",
            "backup.enabled=true",
            "--set",
            &format!("backup.engine={engine}"),
            "--set",
            "backup.tokenSecret.name=sanshain-backup-token",
        ])
        .expect("chart renders the backup CronJob");
        assert!(
            rendered.contains("rm -f /backup/*.part"),
            "a failed {engine} run must not leave a .part file behind forever, got:\n{rendered}"
        );
    }
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

#[test]
fn default_render_pulls_the_release_image_of_this_version() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    // Release tags carry a `v`, so the chart's default tag must too, and the
    // chart's appVersion must follow the crate version at every bump.
    let expected = format!(
        "image: \"ghcr.io/paxel/sanshain-service:v{}\"",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        rendered.contains(&expected),
        "the default image must be the release image of this version: {expected}"
    );
}

// --- 2.6.1 (#58) ---

#[test]
fn default_render_has_no_env_and_no_dead_keys() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    assert!(
        !rendered.contains("\n          env:\n"),
        "no env block without extraEnv, so existing installations are untouched"
    );
    // The service never read these two; the chart stopped claiming otherwise.
    assert!(
        !rendered.contains("PROMETHEUS_ENDPOINT"),
        "got:\n{rendered}"
    );
    assert!(
        !rendered.contains("LOGIN_SESSION_DURATION_HOURS"),
        "got:\n{rendered}"
    );
}

#[test]
fn no_hpa_unless_autoscaling_is_enabled() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    assert!(
        !rendered.contains("HorizontalPodAutoscaler"),
        "an untouched installation gets no HPA"
    );
    assert!(
        rendered.contains("replicas: 1"),
        "the Deployment keeps its replica count without an HPA"
    );
}

#[test]
fn autoscaling_renders_an_hpa_from_the_values() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "autoscaling.enabled=true",
        "--set",
        "autoscaling.minReplicas=2",
        "--set",
        "autoscaling.maxReplicas=6",
        "--set",
        "autoscaling.targetCPUUtilizationPercentage=70",
        "--set",
        "autoscaling.targetMemoryUtilizationPercentage=85",
    ])
    .expect("chart renders with autoscaling enabled");

    assert!(
        rendered.contains("apiVersion: autoscaling/v2"),
        "got:\n{rendered}"
    );
    assert!(rendered.contains("kind: HorizontalPodAutoscaler"));
    assert!(
        rendered.contains("name: t-sanshain\n  minReplicas: 2\n  maxReplicas: 6"),
        "the HPA targets the Deployment with the configured bounds, got:\n{rendered}"
    );
    assert!(
        rendered.contains("name: cpu\n        target:\n          type: Utilization\n          averageUtilization: 70"),
        "the CPU target, got:\n{rendered}"
    );
    assert!(
        rendered.contains("name: memory\n        target:\n          type: Utilization\n          averageUtilization: 85"),
        "the memory target, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("replicas: 1"),
        "the Deployment leaves the replica count to the HPA"
    );
}

#[test]
fn extra_env_is_appended_to_the_container() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "extraEnv[0].name=PUBLIC_BASE_URL",
        "--set",
        "extraEnv[0].value=https://sanshain.corp",
        "--set",
        "extraEnv[1].name=OTEL_EXPORTER_OTLP_HEADERS",
        "--set",
        "extraEnv[1].valueFrom.secretKeyRef.name=otel-headers",
        "--set",
        "extraEnv[1].valueFrom.secretKeyRef.key=headers",
    ])
    .expect("chart renders with extraEnv");

    assert!(
        rendered.contains(
            "          env:\n            - name: PUBLIC_BASE_URL\n              value: https://sanshain.corp"
        ),
        "a name/value entry must reach the container env, got:\n{rendered}"
    );
    assert!(
        rendered.contains(
            "            - name: OTEL_EXPORTER_OTLP_HEADERS\n              valueFrom:\n                secretKeyRef:\n                  key: headers\n                  name: otel-headers"
        ),
        "a valueFrom entry must be passed through, got:\n{rendered}"
    );
    // The ConfigMap and the Secret are still wired in.
    assert!(rendered.contains("configMapRef:"));
    assert!(rendered.contains("secretRef:"));
}

#[test]
fn default_render_omits_the_empty_secret_keys() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[]).expect("chart renders with default values");

    // Before 2.6.1 both keys were rendered with an empty value, which reached
    // the service as INITIAL_ADMIN_PASSWORD="" and DATABASE_URL="".
    assert!(
        !rendered.contains("INITIAL_ADMIN_PASSWORD"),
        "an empty initial admin password must not be rendered, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("DATABASE_URL:"),
        "an empty database URL must not be rendered, got:\n{rendered}"
    );
    assert!(
        rendered.contains("kind: Secret"),
        "the Secret itself still exists for the secretRef"
    );
}

#[test]
fn configured_secret_values_are_rendered_base64_encoded() {
    if !helm_available() {
        eprintln!("skipping: helm is not installed");
        return;
    }

    let rendered = render(&[
        "--set",
        "secrets.databaseUrl=postgres://u:p@db/sanshain",
        "--set",
        "secrets.initialAdminPassword=s3cret",
    ])
    .expect("chart renders with both secrets set");

    // echo -n "postgres://u:p@db/sanshain" | base64 ; echo -n "s3cret" | base64
    assert!(
        rendered.contains(r#"DATABASE_URL: "cG9zdGdyZXM6Ly91OnBAZGIvc2Fuc2hhaW4=""#),
        "got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"INITIAL_ADMIN_PASSWORD: "czNjcmV0""#),
        "got:\n{rendered}"
    );
}
