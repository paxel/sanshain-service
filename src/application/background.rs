//! Failures of the periodic background jobs (cleanups, the webhook worker).
//!
//! A job that fails every run is otherwise only a WARN line repeated each
//! interval, which nobody watches: the Postgres credential cleanup failed
//! hourly for days that way. Each failure also counts in
//! `sanshain_background_job_failures_total{job}`, so monitoring can alert on it.

/// Log a failed run of `job` and count it. `job` is a stable metric label
/// (`credential_cleanup`); `label` is how the log line names the job.
pub fn job_failed(job: &'static str, label: &str, error: &impl std::fmt::Debug) {
    tracing::warn!("{label} failed: {error:?}");
    metrics::counter!("sanshain_background_job_failures_total", "job" => job).increment(1);
}
