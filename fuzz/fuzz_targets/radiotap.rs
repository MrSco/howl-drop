#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_radio::radiotap;

fuzz_target!(|data: &[u8]| {
    if let Some(rx) = radiotap::parse(data) {
        assert!(rx.frame.len() <= data.len());
    }
});
