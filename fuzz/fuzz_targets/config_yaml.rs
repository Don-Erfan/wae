#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    let _ = wae_config::Config::from_yaml(source);
});
