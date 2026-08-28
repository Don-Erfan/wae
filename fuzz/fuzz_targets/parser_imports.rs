#![no_main]

use libfuzzer_sys::fuzz_target;
use wae_core::domain::ModulePath;
use wae_parser::{JsTsParser, ParserAdapter};

fuzz_target!(|source: &str| {
    let _ = JsTsParser.parse_imports(&ModulePath("fuzz.tsx".into()), source);
});
