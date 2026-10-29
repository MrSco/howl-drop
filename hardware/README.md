# Hardware

What's known about each Wi-Fi card, for README.md's "Supported hardware" table, which `cargo xtask hardware` generates from this directory and `crates/owly-radio/drivers.toml`. Edit these, never the table: `cargo xtask hardware --check` (part of `just ci` and CI) fails when the README's table isn't the one they give.

- `cards.toml`: the cards the table lists, with their PCI or USB IDs, status and the machines it was tested on. Its header explains the fields.
- `reports/`: reports from `owly test-card`, one JSON file each. A card keeps its status from `cards.toml` until a report here gives it a new one.

## Adding a report

1. On the machine with the card, run `owly test-card` and follow it through a photo each way. It writes `owly-test-<driver>-<time>.json` in the current directory (`--out` puts it elsewhere). If a card fails where it should work, restart the iPhone and run again before reporting: a stuck phone fails the same way as a broken card.
2. Copy that file into `reports/` unchanged. It names the card, driver, kernel, distribution, mode and each step's result, and nothing that identifies the network: no router MAC, SSID or address.
3. If the report's device ID isn't on a card in `cards.toml` yet, add it to its card's `ids`, or add the card. The generator refuses a report it can't place.
4. Run `cargo xtask hardware` and commit the report, `cards.toml` and README.md together.

`xtask/src/hardware.rs` documents how reports become a status: which report counts, when a card is ✅, ⚠️, ❌ or ❓, and which machines are listed.
