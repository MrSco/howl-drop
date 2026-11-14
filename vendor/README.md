# Patched dependencies

Copies of crates.io releases with a fix the release lacks, wired in with
`[patch.crates-io]` in the workspace `Cargo.toml`. Each is the published
crate unchanged apart from the lines marked `HowlDrop:`. Drop the copy once
a release has the fix.

## smoltcp 0.13.1

The retransmission timer, in `src/socket/tcp.rs`. A PC sending to an
iPhone over AWDL loses frames often (the phone leaves the channel), and
every fast retransmit doubled the retransmission timeout, so after a few
holes a single lost segment at the end of an upload waited up to 60 s, past
the iPhone's patience. Only a timeout backs the timer off now, and it stops
at 4 s.

smoltcp also asked for SACK (RFC 2018) but ignored the blocks the receiver
sent, so a retransmission resent everything after a lost segment: some 80%
of an upload's frames were data the iPhone already had. A small scoreboard
(`Scoreboard`) keeps the reported ranges, and sending skips them; a timeout
forgets them and resends everything.

Skipping reported ranges still resent what was in flight: a fast
retransmission rewound to the first gap and went on through the data the
phone had not yet had time to report, most of the window on AWDL's long
round trip. Now, while the remote sends SACK, a fast retransmission goes
at once, resends only the gaps below the highest reported byte, and then
carries on from where sending had got to (`recover`, `next_to_send`).

Both copies' `Cargo.toml` allow the upstream code's warnings.

## tokio-smoltcp 0.6.0

`Reactor::notify` used `Notify::notify_waiters`, which wakes the reactor only
if it is already waiting and remembers nothing otherwise. A socket write that
landed while the reactor was between polls (computing its next deadline,
sending, polling the interface) was not sent until another packet arrived or
the reactor's 60 s idle timer fired. Uploads to an iPhone stalled for up to a
minute whenever the phone had acknowledged everything and had nothing to
send. `notify_one` stores a permit for the next wait instead.
