//! An AsyncAPI provide: the split into channels, the published and subscribed
//! messages, the unnamed ones and the server URLs, as a provide runs them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain_service::asyncapi;

fuzz_target!(|data: &[u8]| {
    let Ok(spec) = std::str::from_utf8(data) else {
        return;
    };
    let _ = asyncapi::absolute_server_urls(spec);
    if let Ok(channels) = asyncapi::split_asyncapi(spec) {
        for channel in &channels {
            let _ = asyncapi::channel_summary(&channel.yaml_content);
        }
    }
    let _ = asyncapi::extract_pub_messages(spec);
    let _ = asyncapi::extract_sub_messages(spec);
    let _ = asyncapi::unnamed_messages(spec);
    let _ = asyncapi::check_backward_compatibility(spec, spec);
});
