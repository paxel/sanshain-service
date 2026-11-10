//! A proto provide: the version marker, the split into rpcs and the comparison
//! with itself, as a provide runs them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain_service::proto;

fuzz_target!(|data: &[u8]| {
    let Ok(spec) = std::str::from_utf8(data) else {
        return;
    };
    let _ = proto::has_version_marker(spec);
    let _ = proto::extract_sanshain_version(spec);
    if let Ok(rpcs) = proto::split_proto(spec) {
        for rpc in &rpcs {
            let _ = proto::rpc_summary(&rpc.content);
        }
    }
    let _ = proto::check_backward_compatibility(spec, spec);
});
