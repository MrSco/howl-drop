#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_airdrop::proto::http::{BodyDecoder, Limits, parse_request, parse_response};

/// Decodes a body, fed in pieces of `step` bytes.
fn body(decoder: &mut BodyDecoder, input: &[u8], step: usize) {
    let mut buf = Vec::new();
    for piece in input.chunks(step.max(1)) {
        buf.extend_from_slice(piece);
        loop {
            match decoder.decode(&buf) {
                Ok(s) => {
                    assert!(s.data.end <= s.consumed && s.consumed <= buf.len());
                    buf.drain(..s.consumed);
                    if s.consumed == 0 || decoder.is_done() {
                        break;
                    }
                }
                Err(_) => return,
            }
        }
    }
    let _ = decoder.finish();
}

fn run(data: &[u8], limits: &Limits, max_body: u64) {
    let step = data.first().copied().unwrap_or(1) as usize;
    if let Ok(Some((head, n))) = parse_request(data, limits) {
        if let Ok(mut decoder) = BodyDecoder::new(head.body, max_body, limits) {
            body(&mut decoder, &data[n..], step);
        }
    }
    if let Ok(Some((head, n))) = parse_response(data, limits) {
        if let Ok(mut decoder) = BodyDecoder::new(head.body, max_body, limits) {
            body(&mut decoder, &data[n..], step);
        }
    }
}

// spec: SEC-3, SEC-7
fuzz_target!(|data: &[u8]| {
    run(data, &Limits::default(), 1 << 20);
    // Again with limits scaled to the input, so the fuzzer's small inputs
    // reach the head, header, trailer and body limits.
    let n = data.len();
    let small = Limits {
        max_head: n / 4,
        max_headers: 1 + n % 8,
    };
    run(data, &small, n as u64 / 4);
});
