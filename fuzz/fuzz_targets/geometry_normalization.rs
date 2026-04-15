#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kaidoku_core::fuzzing::fuzz_geometry_path(data);
});
