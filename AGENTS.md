# AGENTS.md

## The spec

`docs/SPEC.md` specifies the AirDrop protocol on its own, the way the vendor would write it, and Owly's crates implement it. Keep it that way:

- State the protocol, not where it was seen: no sources, tags, dates, OS versions or captures in it. Those go in the commit or the capture (`tests/captures/`) that recorded them.
- No implementation detail: nothing about Owly, OWL, Linux, other libraries, file paths or hardware. What Owly does to cope with its radios and OS goes in `docs/implementation.md`, with `OWLY-n` IDs where a test needs to cite a rule.
- Something not yet known is written as a safe rule (*reserved*, *not defined in this version*) and listed in `docs/open-questions.md`.
- Requirement IDs are cited across the code and in `tests/coverage.toml`, so they never change or get reused: a removed requirement stays as `**ID** *withdrawn*`. Section numbers are cited too, so don't renumber them.
- `cargo xtask coverage` reads requirement IDs from `docs/SPEC.md` and `docs/implementation.md`, and the `T-*` test table from `docs/conformance.md`.

## Legacy

"Legacy" in code and docs means `omdrop-owl` 0.3.0 and older, the patched OWL and the shell and Python around it that Owly replaces. Its code is at the `v0.3.0` tag: read a file it cites, such as `patches/owl-04-election-metric.patch`, with `git show v0.3.0:<path>`.

## README hardware table

The "Supported hardware" table in README.md is generated: `cargo xtask hardware` writes it between the `<!-- hardware-table:start -->` and `<!-- hardware-table:end -->` lines, from `hardware/cards.toml`, the `owly test-card` reports in `hardware/reports/`, and `crates/owly-radio/drivers.toml`. Don't edit the table by hand: change those files and run it. `cargo xtask hardware --check` (part of `just ci`) fails when the README's table is out of date.

The generator keeps these rules, so keep them in mind when changing it or the data:

- Rows are grouped by status: ✅, then ⚠️, then ❌, then ❓. Within each group, MediaTek, Realtek and Qualcomm cards come before Intel, since `iwlwifi` only supports exclusive mode.
- Only list a machine on a card's row if it tells readers something new: a device ID or mode result not yet listed, or a failure. Another machine with a listed device ID and the same result on a different distribution or kernel is redundant. The generator lists one machine per device ID, mode and result, so a report like that changes nothing.
- A card's status in `hardware/cards.toml` stands until an `owly test-card` report gives it a new one. The machines listed there stay listed beside the reports' machines.

`hardware/README.md` says how to add a report.
