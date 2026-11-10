//! An OpenAPI provide: the version, the split into endpoints, the server URLs
//! and the comparison with itself, as a provide runs them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain_service::openapi;

fuzz_target!(|data: &[u8]| {
    let Ok(spec) = std::str::from_utf8(data) else {
        return;
    };
    let _ = openapi::extract_info_version(spec);
    let _ = openapi::absolute_server_urls(spec);
    if let Ok(endpoints) = openapi::split_openapi(spec) {
        for endpoint in &endpoints {
            let _ = openapi::operation_summary(&endpoint.yaml_content);
        }
    }
    let _ = openapi::check_backward_compatibility(spec, spec);
    let _ = openapi::analyze_impact(spec, spec);
});
