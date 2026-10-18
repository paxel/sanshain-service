//! Startup refuses configuration that would otherwise crash the service later.
//!
//! `CLEANUP_INTERVAL_SECS=0` used to pass the number check and panic when the
//! cleanup timer was created, right after the service had logged its start.
//! The check is inside `main`, so the binary is started for real, against a
//! throwaway SQLite file, from the repository root (the migrations are read
//! from there); it must exit before it binds, with the variable named.

use std::path::{Path, PathBuf};
use std::process::Command;

fn throwaway_sqlite() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "sanshain-startup-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

#[test]
fn cleanup_interval_zero_is_refused_at_startup() {
    let dir = throwaway_sqlite().expect("a throwaway directory");
    let db = format!("sqlite:{}/sanshain.db?mode=rwc", dir.display());

    let output = Command::new(env!("CARGO_BIN_EXE_sanshain_service"))
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .env_remove("PUBLIC_BASE_URL")
        .env("DATABASE_URL", &db)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("OTEL_ENABLED", "false")
        .env("CLEANUP_INTERVAL_SECS", "0")
        .output()
        .expect("the service binary starts");
    let _ = std::fs::remove_dir_all(&dir);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a configuration error exits with 1, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("CONFIGURATION ERROR: CLEANUP_INTERVAL_SECS must be greater than 0."),
        "the variable is named on stderr, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "refused, not crashed, got:\n{stderr}"
    );
}
