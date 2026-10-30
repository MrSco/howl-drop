# owly-awdl

AWDL (Apple Wireless Direct Link), the Wi-Fi link AirDrop runs over, as a sans-IO state machine. It's the AWDL in [Owly](https://github.com/t4t5/owly), which runs AirDrop on ordinary Linux Wi-Fi cards.

`Node` takes what the radio received, the IPv6 packets the host wants sent and timer expiries, each with the time, and returns what to do: transmit a frame, arm the timer, add or remove a peer, deliver a packet. It does no I/O and reads no clock, so it runs the same from tests, captures and simulations as on a radio. It's `no_std`, with `alloc`.

The modules follow §3 of [docs/SPEC.md](https://github.com/t4t5/owly/blob/main/docs/SPEC.md): `frame`, `tlv`, `sync`, `election`, `channel`, `peer`, `ip` and `rate`.

## The driver

The `driver` feature adds `driver::spawn`, which runs a node on a thread of its own with tokio, on any radio that can inject and capture 802.11 frames. The radio implements two traits: `Air`, where the node's frames go out and the peers' come in, and `Netif`, the host's network interface for the link, where IPv6 packets come and go. [owly-radio](https://crates.io/crates/owly-radio) implements them on Linux, with a monitor interface and a TAP device. A radio elsewhere, such as an ESP32 over USB, needs only an `Air` of its own.

## Status

On 0.x until Owly 1.0: the API can change between minor versions.

## License

MIT
