//! A failed background job is counted, not only logged.

use sanshain_service::application::background::job_failed;

#[tokio::test]
async fn a_failed_job_counts_in_the_failure_metric() {
    let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
    job_failed("credential_cleanup", "Credential cleanup", &"boom");
    job_failed("credential_cleanup", "Credential cleanup", &"boom");
    job_failed("webhook_cleanup", "Webhook cleanup", &"boom");
    let text = handle.render();
    assert!(
        text.contains(r#"sanshain_background_job_failures_total{job="credential_cleanup"} 2"#),
        "{text}"
    );
    assert!(
        text.contains(r#"sanshain_background_job_failures_total{job="webhook_cleanup"} 1"#),
        "{text}"
    );
}
