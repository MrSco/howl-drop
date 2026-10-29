# Capture corpus

Real AWDL sessions between this repo's Linux stack and Apple devices, as pcap files of radiotap frames, each with a `.toml` of what replaying it must find. `crates/owly-radio/tests/replay.rs` feeds every capture here through owly-awdl's `Node` and checks that every AWDL action frame parses whole (T-AWDL-1), that the peers the `.toml` names turn valid, that the node follows the master it names, and that it keeps within 3 TU of that master's schedule (T-AWDL-3). Still wanted: an iPhone restart mid-session, zero-padded MIFs (AWDL-5), A-MSDU (AWDL-14) and ADDBA (AWDL-15), and every session on each social channel.

## Recording one

With the radio up in one terminal (`just radio`), find the phone's AWDL address with `just owly radio peers`, then in another:

```sh
sudo target/debug/owly capture receive --ios 26.1 --peer 86:08:04:8a:2b:f4
```

Do what the capture is meant to show, and stop it with Ctrl-C (or give `--seconds`). It writes `<what>-ios<version>-<driver>-ch<channel>-<n>.pcap` and its `.toml` here.

## What's in a capture, and what isn't

The corpus is committed, so `owly capture` takes everything personal out before it writes anything; until then the frames are only in memory:

- It keeps only AWDL frames to and from the `--peer` addresses and this computer's `awdl0`: nothing of the router's, of other Wi-Fi networks', or of other Apple devices nearby.
- Every MAC address becomes a stand-in, `02:00:00:00:00:01` and on, wherever it appears: frame headers, the masters and sync trees in TLVs (your other Apple devices), the infrastructure BSSID and address a phone announces in its Data Path State (your router's, which would place you), and the interface IDs of IPv6 addresses. Stand-ins keep the order of the real addresses, so elections replay the same. Which stand-in is which device is printed on the terminal, and written nowhere.
- Host names (ARPA TLVs) become `x`s, and the country code `XX`. TLVs whose contents aren't known (service records, which carry mDNS names, the bloom filter, deprecated types) are zeroed.
- Every packet keeps its IPv6 header and its TCP, UDP or ICMPv6 header; the rest is zeroed, mDNS names and whatever was sent with them included.
- The radiotap header has only the channel and the signal strength, and time counts from the first frame, with no date.

Lengths stay as they were, so every frame parses as it did. The replay test refuses a capture that isn't anonymous: an address or a `.toml` entry that isn't a stand-in, a wall-clock time, more radiotap than channel and signal, a host name.

What does remain: how the phone behaves on the air (its schedule, election metrics, channel sequences, capabilities and timing), the iOS version and Wi-Fi driver in the file name, and the sizes and timing of the packets.

## The `.toml`

```toml
channel = 44                      # the channel it was recorded on
local = "02:00:00:00:00:03"       # this computer's awdl0, if it was up
peers = ["02:00:00:00:00:02"]     # stand-ins; each must turn valid
master = "02:00:00:00:00:02"      # optional: the master a passive node follows at the end
notes = "receive on iOS 26.1, mt7925e"
```

`owly capture` fills in all but `master`, which only someone who knows the session can confirm: the terminal shows which stand-in is which device.
