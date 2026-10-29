#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_airdrop::proto::plist::{Limits, from_bytes, to_bytes};

// spec: SEC-3, SEC-6
fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    if let Ok(value) = from_bytes(data, &limits) {
        // Whatever decodes re-encodes to something that decodes the same.
        let again = from_bytes(&to_bytes(&value), &Limits { max_size: usize::MAX, ..limits }).unwrap();
        assert_eq!(format!("{value:?}"), format!("{again:?}"));
    }
    // Again with limits scaled to the input, so the fuzzer's small inputs
    // reach every one of them.
    let n = data.len();
    let small = Limits {
        max_size: n,
        max_depth: 1 + n % 8,
        max_values: 1 + n / 16,
        max_bytes: n / 4,
    };
    let _ = from_bytes(data, &small);
});
