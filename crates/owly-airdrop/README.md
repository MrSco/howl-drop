# owly-airdrop

AirDrop, as iPhones, iPads and Macs speak it: discovery, the request to send, and the upload. It's the AirDrop layer of [Owly](https://github.com/t4t5/owly), and runs over any IPv6 link to the peer, such as the `awdl0` that [owly-radio](https://crates.io/crates/owly-radio) brings up.

- `proto` is the sans-IO core: a strict binary plist reader and writer, HTTP/1.1 framing, cpio, gzip and DVZip archives with the rules for extracting them, the messages, the BLE advert and mDNS records, and the sender and receiver state machines. Nothing in it touches a socket, a file or a clock.
- The `tokio` feature, on by default, adds `driver`: a sender and a receiver over TLS (rustls) with a self-signed certificate, which extract uploads into a sandboxed staging directory.
- The `mdns` feature adds an mDNS/DNS-SD responder and browser for `_airdrop._tcp` on one interface, for hosts without a system responder.

The driver doesn't choose rustls's crypto provider: the application installs one, such as `rustls::crypto::ring::default_provider().install_default()`.

The receiver works in Everyone mode. Contacts Only needs an Apple identity, which this crate doesn't do.

[docs/SPEC.md](https://github.com/t4t5/owly/blob/main/docs/SPEC.md) says what it implements on the wire.

## Status

On 0.x until Owly 1.0: the API can change between minor versions.

## License

MIT
