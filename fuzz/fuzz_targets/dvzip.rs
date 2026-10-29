#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_airdrop::proto::archive::compress::{Compressor, Decoder};

// spec: DVZ-3, SEC-3
fuzz_target!(|data: &[u8]| {
    // A chunk limit the fuzzer's small inputs can reach.
    let mut decoder = Decoder::new(Compressor::DvZip, data.len() as u32 / 4);
    let mut out = [0; 4096];
    let mut input = data;
    let mut produced = 0;
    // Stop past 16 MiB: bounding output is the caller's job (SEC-3).
    while produced < 16 << 20 {
        match decoder.decode(input, &mut out) {
            Ok(p) => {
                assert!(p.consumed <= input.len() && p.produced <= out.len());
                input = &input[p.consumed..];
                produced += p.produced;
                if p.consumed == 0 && p.produced == 0 {
                    return;
                }
            }
            Err(_) => return,
        }
    }
});
