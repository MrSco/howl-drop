# Importing a sending identity

Anonymous sending requires the iPhone's AirDrop to be on **Everyone for 10
Minutes**. An Apple-issued sending identity supplies the contact hashes,
signed validation record and TLS client certificate needed for **Contacts
Only**. The iPhone must know one of the identity's verified email addresses
or phone numbers as a contact. Importing an identity does not add that
contact or bypass the receiver's policy.

This is source-build support: use an app and agent built from the same
revision. Import validation and protocol integration have automated tests;
Contacts Only sending has not yet been verified against an iPhone with a
real imported identity. Do not treat a successful import as an
interoperability test.

## Input

Obtain these three files for your own Apple account through an export you
trust, in one directory:

| File | Contents |
|---|---|
| `certificate.pem` | Apple-issued identity certificate, leaf first, optionally followed by its PEM certificate chain. |
| `key.pem` | Matching **unencrypted** PEM private key (PKCS#8, RSA PKCS#1 or EC SEC1). An encrypted key or `.p12` is not accepted directly. |
| `validation_record.cms` | Original Apple-signed CMS validation record for the same account. Keep the signed bytes intact. |

HowlDrop does not extract these files, generate an Apple identity, or sign
validation records. The old `seemoo-lab/airdrop-keychain-extractor` expects
an account archive that was absent on the Mac examined during development;
its instructions also disable SIP/AMFI. It is not a supported setup route.
The current macOS export path remains unresolved. No security protections
need to be changed to check or import files already available.

Each input file is limited to 64 KiB. The importer checks that the key
matches the leaf certificate, verifies the certificate and CMS signatures
against the bundled Apple root and issuers, binds both to the same account,
and requires a current certificate and a dated, unexpired validation
record. As with receiving, issuer certificate dates are not enforced; the
identity's leaf certificate dates are. Contact hashes must be SHA-256 hex
strings, with 1–128 distinct two-byte prefixes.

## Windows

Build the agent from the repository root, then check the exported directory
without touching the radio or writing an installed identity:

```powershell
cargo build --release --locked -p howldrop-agent
$agent = Join-Path $PWD 'target\release\howldrop-agent.exe'
& $agent --check-sender-identity 'C:\Private\AirDropIdentity'
```

Import as your ordinary Windows user, the same user who runs HowlDrop:

```powershell
$bundle = Join-Path $env:LOCALAPPDATA 'HowlDrop\sender-identity.bin'
& $agent --import-sender-identity 'C:\Private\AirDropIdentity' --sender-identity $bundle
& $agent --check-sender-identity $bundle
```

Check each command's exit code: success is zero. Import validates everything
before atomically replacing the destination; rejected input preserves an
existing identity. Check/import commands need neither a stick nor
`--staging` and exit without starting AWDL or Bluetooth.

The bundle uses user-scoped Windows DPAPI encryption. It is not a portable
backup: another user or machine cannot simply load the file. The original
export still contains an unencrypted private key; keep it private and
remove disposable copies when they are no longer needed. Neither command
prints keys, account identifiers or contact hashes.

For a local app build, follow [native-windows.md](native-windows.md#radio-examples)
to place the matching agent and firmware beside `howldrop.exe`. Restart
HowlDrop when ready. The tray automatically uses the bundle at the path
above; a standalone agent instead needs `--sender-identity FILE` with its
usual receiving/sending arguments. The tray advertises verified short
contact prefixes while looking for devices or sending, rotating groups
every two seconds if there are more than four. It sends the signed record
on `/Discover` and `/Ask` and presents the matching TLS client certificate.
The BLE metadata travels over the tray's private agent pipe and is omitted
from its log.

## Expiry, refresh and removal

Validity ends at the earlier of the record's expiry and the identity
certificate's expiry. The check command prints that time as Unix seconds.
Import a fresh export to the same destination, then restart HowlDrop to
load it. Sending checks expiry again before starting work; the Windows
tray stops advertising contact hashes once expired.

An invalid, expired or undecryptable configured bundle stops the agent at
startup with an identity error. To return to anonymous sending, turn off
**AirDrop on this PC**, move `sender-identity.bin` out of the HowlDrop data
directory, and turn it back on. The iPhone then needs **Everyone for 10
Minutes**. These steps leave the separate receiving identity and contact
list alone.

## Linux

The same check and import options work with `howldrop-agent`. Supply
`--sender-identity FILE` explicitly when running the agent. The installed
bundle is plaintext, owner-readable only (mode `0600`); loading refuses
group/other permissions, and newly created parent directories use `0700`.
Windows DPAPI bundles cannot be copied to Linux; import from the export
files there instead.

With `--ble`, the current BlueZ path advertises the first four verified
prefixes, without the Windows tray's rotation. Contacts known only by a
later prefix may not wake the receiver. Neither this path nor the Windows
path has had a real-identity Contacts Only hardware test yet.
