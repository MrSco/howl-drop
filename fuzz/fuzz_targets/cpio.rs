#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_airdrop::proto::archive::cpio::{Event, Reader};

// spec: UP-2, SEC-1
fuzz_target!(|data: &[u8]| {
    let mut reader = Reader::new();
    let mut input = data;
    loop {
        match reader.feed(input) {
            Ok((n, event)) => {
                assert!(n <= input.len());
                input = &input[n..];
                if matches!(event, Some(Event::Trailer)) || (n == 0 && event.is_none()) {
                    return;
                }
            }
            Err(_) => return,
        }
    }
});
