# HowlDrop's fork of Owly

HowlDrop is a fork of [t4t5/owly](https://github.com/t4t5/owly) (formerly
omdrop-owl). Since 0.19.0 it keeps Owly's tree as upstream has it, so each
new Owly merges in, and HowlDrop's own code lives beside it:

- `crates/howldrop-agent`: the Linux side (radio, receiver, sender, the
  events howldrop.exe reads), on Owly's crates as libraries. It runs the
  radio itself, without `owlyd`.
- `howldrop/`, `addon/`, `docker/`, `tools/`, `art/`, `site/` (the
  GitHub Pages site), `.github/` (`README.md`, `workflows/howldrop.yml`,
  `workflows/pages.yml`), and `docs/adapters.md`,
  `hardware-notes.md` and this file.

Changes go to MrSco/howl-drop only, not upstream.

## Patches to Owly

Each is a commit of its own, with tests. They're small on purpose: the less
of Owly that changes, the less a merge has to resolve. Each patch leaves Owly's
behaviour as it was unless asked for (an option, a new method), except the
401, the HT40 tuning and the reordering (on where Block Ack sessions are
accepted), which change it for everyone.

| Crate | What | Why |
|---|---|---|
| owly-airdrop | A declined or unanswered `/Ask` gets `401` (`receiver.rs`, `http.rs`, SPEC ASK-2) | An iPhone shows a 401 as "Declined"; with 403 it stayed on "Waiting". |
| owly-airdrop | `Options.progress`: `ReceiverEvent::Progress` while an upload comes in, from 0 as it starts | The cards' progress bar, and how long a transfer took. |
| owly-airdrop | `Request.peer_certificate`: the sender's TLS client certificate | Contacts Only: the agent ties the Apple-signed record to the sender's account. |
| owly-airdrop | `Config.discover_gate`: refuse a `/Discover` with 401 | Contacts Only: strangers don't see the PC. |
| owly-airdrop | sender `Config.file_icon` and `Sender::send_with_icon` | The preview an iPhone shows for a picture sent to it. |
| owly-awdl | `reorder.rs`: a Block Ack session's frames put back in order | Out-of-order segments make TCP resend. (It was not what made uploads slow: the next one was.) |
| owly-radio | `set_freq` tunes HT40+ on 5 GHz, else HT20 | The node announces 40 MHz and the iPhone sends at HT40: tuned HT20, the MT7612U heard a fraction of an upload. |

Outside the crates: two lines in `.gitignore` (`/build/`, `*.exe`).

`git diff upstream/main HEAD -- $(git ls-tree -r --name-only upstream/main)`
lists every change to upstream's files.

## Taking in a new Owly

```bash
tools/sync-upstream.sh
```

It fetches `upstream` (t4t5/owly), merges `upstream/main` into the current
branch, and runs the Rust and Python tests. Do it on a branch. When the merge
stops on a conflict, it is in one of the patches above: keep upstream's
change and put ours back on top, then `git merge --continue` and run the
script's tests again (`cargo test --workspace`, `python3 -m unittest
discover -s tests`).

Then check what upstream changed that HowlDrop uses: `drivers.toml` (cards,
modes, channels), the receiver and sender APIs the agent calls, and the
radio's set-up. A card upstream adds works in the add-on at once; on Windows
it also needs its driver in WSL's kernel and its firmware (see
[adapters.md](adapters.md)).

Upstream's workflows (`.github/workflows/ci.yml`, release, AUR) stay as they
are: the release ones run only in t4t5/owly, and ci.yml tests the crates
here too. HowlDrop's own CI is `howldrop.yml`.
