//! A catalog push: the Component named like the system, out of the pushed
//! catalog-info.yaml.
#![no_main]

use libfuzzer_sys::fuzz_target;
use sanshain_service::backstage;

fuzz_target!(|data: &[u8]| {
    let Ok(catalog_info) = std::str::from_utf8(data) else {
        return;
    };
    let _ = backstage::read_component(catalog_info, "orders");
});
