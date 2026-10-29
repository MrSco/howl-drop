#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use owly_airdrop::proto::receiver::{Config, Connection, Event};

// The whole receiver, fed hostile bytes: whatever arrives, it answers with an
// error at worst, and never panics (SEC-8).
// spec: SEC-8
fuzz_target!(|data: &[u8]| {
    // Limits scaled to the input, so the fuzzer's small inputs reach them.
    let n = data.len();
    let mut config = Config::new("Fuzz");
    let limits = &mut config.limits;
    limits.http.max_head = 64 + n / 4;
    limits.http.max_headers = 2 + n % 8;
    limits.plist.max_size = n / 2;
    limits.plist.max_values = 1 + n / 32;
    limits.plist.max_bytes = n / 4;
    limits.max_upload_body = n as u64 / 2;
    limits.extract.max_entries = 1 + n as u32 % 8;
    limits.extract.max_total = n as u64 / 2;
    limits.max_dvzip_chunk = n as u32 / 4;
    limits.max_buffered = n / 4;
    let mut conn = Connection::new(Arc::new(config));
    // The first byte picks how the input is split into reads.
    let step = data.first().copied().unwrap_or(1).max(1) as usize;
    for piece in data.chunks(step) {
        conn.receive(piece);
        while let Some(event) = conn.poll() {
            match event {
                Event::Ask(_) => conn.accept(),
                Event::UploadComplete => conn.finish_upload(Ok(())),
                Event::Close => return,
                _ => {}
            }
        }
    }
    conn.receive_eof();
    while let Some(event) = conn.poll() {
        if event == Event::UploadComplete {
            conn.finish_upload(Ok(()));
        }
    }
});
