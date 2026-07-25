#![no_main]

use executive::composition::config::AppConfig;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        let _ = toml::from_str::<AppConfig>(source);
    }
});
