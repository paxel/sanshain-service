//! A Product's pasted umbrella Chart.yaml: its dependencies' names.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain_service::helm;

fuzz_target!(|data: &[u8]| {
    let Ok(chart) = std::str::from_utf8(data) else {
        return;
    };
    let _ = helm::dependency_names(chart);
});
