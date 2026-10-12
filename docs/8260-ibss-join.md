# 8260 IBSS join: what returned EOPNOTSUPP

0.1.4 on the NUC. After `wlp3s0` was found and host `wpa_supplicant` was stopped, the second join ran on `awdlibss0` once that interface was up:

```
iw dev awdlibss0 ibss join omdrop 2437 HT20 fixed-freq 00:25:00:ff:94:73 beacon-interval 10000
```

`iw` printed `command failed: Operation not supported (-95)`. That errno is `EOPNOTSUPP`. The earlier `Network is down (-100)` is a down interface and is not this finding. 0.1.5 sets the netdev to type ibss while it is down, brings it up, then runs this same join. The join arguments stay as they are. `iw dev info` prints that type as `IBSS`. 0.1.5 compared it with the lowercase word `ibss` and logged a refusal after `set type` had succeeded. 0.1.6 accepts `IBSS`.

Home Assistant OS on this NUC is the generic x86-64 image, Linux 6.12. The call below is `NL80211_CMD_JOIN_IBSS` in that tree (`net/wireless/nl80211.c`, `nl80211_join_ibss`).

## What `iw` sends

Debian `iw` encodes that command as one `NL80211_CMD_JOIN_IBSS`:

| Command word | Netlink attribute |
| --- | --- |
| `omdrop` | `NL80211_ATTR_SSID` |
| `2437` | `NL80211_ATTR_WIPHY_FREQ` 2437, `NL80211_ATTR_CENTER_FREQ1` 2437 |
| `HT20` | `NL80211_ATTR_CHANNEL_WIDTH` = `NL80211_CHAN_WIDTH_20` and `NL80211_ATTR_WIPHY_CHANNEL_TYPE` = `NL80211_CHAN_HT20` |
| `fixed-freq` | `NL80211_ATTR_FREQ_FIXED` |
| `00:25:00:ff:94:73` | `NL80211_ATTR_MAC` |
| `beacon-interval 10000` | `NL80211_ATTR_BEACON_INTERVAL` = 10000 |

`00:25:00:ff:94:73` is a valid unicast address. 2437 MHz is an integer channel, so the channel `freq_offset` is 0.

## Which check returns -95

In `nl80211_join_ibss` the beacon interval is read first. `cfg80211_validate_beacon_int` accepts 10 through 10000 TU inclusive. 10000 passes. A value outside that range returns `-EINVAL` (`-22`), not `-EOPNOTSUPP`.

The only `-EOPNOTSUPP` returns in that function are:

1. The driver has no `join_ibss` operation.
2. The netdev is not `NL80211_IFTYPE_ADHOC`.

Both of those run before the BSSID is read and before HT20 is checked. iwlmvm sets `join_ibss` to `iwl_mvm_start_ibss` and sets `NL80211_IFTYPE_ADHOC` in `interface_modes`. On this card the missing-op check does not fire. The `-95` `iw` printed is check 2: `JOIN_IBSS` was issued on a netdev whose type was not ad-hoc.

HT20 and the fixed BSSID are not that return.

- HT20 is `NL80211_CHAN_WIDTH_20`. If `NL80211_FEATURE_HT_IBSS` is missing, the handler returns `-EINVAL`. iwlmvm sets `NL80211_FEATURE_HT_IBSS` for every mvm device, including the 8000 family, and also sets `NL80211_EXT_FEATURE_VHT_IBSS`.
- The fixed BSSID is copied when `is_valid_ether_addr` succeeds. A bad address is `-EINVAL`. `NL80211_ATTR_FREQ_FIXED` is stored as `channel_fixed`. The nl80211 comment says a fixed BSSID or frequency may be rejected. The 6.12 handler does not reject either one with `-EOPNOTSUPP`.
- mac80211 `ieee80211_ibss_join` returns `-EOPNOTSUPP` only when the channel’s `freq_offset` is non-zero. 2437 MHz does not have one. Otherwise it returns 0 and queues the IBSS work.

So beacon-interval 10000, HT20, and `00:25:00:ff:94:73` are not the argument that produced `-95`. The refused check is the interface type: the join ran on a netdev that was not already IBSS.

## What the 8000C firmware does here

`iwl_mvm_mac_ctxt_cmd_ibss` copies `beacon_int` into `iwl_mac_data_ibss.bi`, a `__le32`. 10000 fits. There is no driver reject of 10000 TU, of HT20, or of a fixed BSSID on the way into that command. The station-only guard that refuses a beacon interval under 16 TU returns failure for association, not `-EOPNOTSUPP`, and 10000 is above it.

That MAC-context command is not what `iw` waited on. `ieee80211_ibss_join` returns 0 first. The firmware command runs later from the IBSS work. `iwl_mvm_mac_ctxt_send_cmd` turns a firmware NACK into `-EIO`, which `iw` would not print as `Operation not supported (-95)`.

Interface combinations on iwlmvm list station, AP, and P2P. They do not list ad-hoc. A second interface beside a live station fails that check with `-EBUSY`, not `-95`. One IBSS interface alone is still allowed, because mac80211 skips the combination list when it is the only interface. The 8260 driver still advertises IBSS and still has `join_ibss`.

0.1.6 set `wlp3s0` to type ibss while it was down, then `ip link set wlp3s0 up` returned `Device or resource busy`. That is this combination check: another interface on the phy was still running, so the single-interface exception did not apply. 0.1.7 deletes every other interface on that phy before the bring-up. The join arguments stay as they are.

## What not to change

Do not drop HT20, do not drop `fixed-freq`, do not drop `00:25:00:ff:94:73`, and do not shorten the beacon interval to make `iw` succeed. AWDL needs that BSSID and that 10000 TU interval. None of those is the `-95`.

A driver patch is not what adds this join. iwlmvm already accepts the three AWDL arguments. The closed `iwlwifi-8000C-36.ucode` image is not what returned `-95`, so patching that blob would not clear this error either. A later firmware NACK of the IBSS MAC context, if one shows up in the kernel log, would be `-EIO` from that closed image, and that image cannot be changed. This log line is not that NACK.

## 0.1.11 frequency order

HT20, `fixed-freq`, BSSID `00:25:00:ff:94:73`, and beacon interval 10000 stay. 0.1.11 runs that join at 5220 MHz and then 5745 MHz before 2437 MHz, which is the order the released exclusive profile already uses. A frequency the phy marks disabled, No IR, or radar is skipped. 2437 MHz remains the fallback when 5 GHz does not come up. The beacon interval is not shortened.

0.1.11 on this NUC skipped 5220 and 5745 because the phy marked them disabled or No IR, then joined 2437. 0.1.12 read the regulatory domain as 00, self-managed, then reported a 5 GHz channel usable and skipped both the country change and the scan. The join still found 5220 and 5745 closed. 0.1.13 sets country 00 to US, does not treat an unreadable channel list as usable, and scans again after the IBSS interface is up. The join arguments stay the same.

## 0.2.2 on channel 44, with OWL's log visible

With the regulatory unlock, the cell joined 5220 MHz and OWL ran pinned to
44, where the iPhone (on 5 GHz home Wi-Fi) spends most of its slots. OWL
read the phone's channel sequence from one frame, then rejected the rest:
`awdl_action: unexpected bytes (2) at end of frame` and `unhandled frame
(-1)`, dozens a second. It never logged `add peer`. On channel 6 the same
phone's frames parse and the peer is added within a second. About 12 s in,
injection failed with `Network is down`, then `No such device or address`:
mon0 no longer existed, and nothing in the add-on or helper deletes it
mid-window. 0.2.3 prints the host kernel's iwlwifi lines when that happens,
to tell a firmware crash from anything else.

On channel 6 the phone's sequence was `44,44,149,0,0,0,0,44,6,44,149,44,0,0,0,44`:
one slot in sixteen. Every exchange with it waits for that slot.

## 0.2.3: the firmware fails on channel 44

The same run with the kernel log printed. About 13 s after the join every
firmware command returned -5 (`Failed to send MAC_CONTEXT_CMD (action:3): -5`,
`PHY ctxt cmd error. ret=-5`), and iwlwifi's reprobe work removed the card
(`iwl_trans_reprobe_wk` → `iwl_pcie_gen1_2_remove` → `ieee80211_unregister_hw`)
and probed it again as a new phy, renaming its `wlan0` to `wlp3s0`. That is
what took mon0 away. `iwlwifi-8000C-36.ucode` is the last firmware this card
gets, so channel 44 is out on the 8260; channel 6 is the only AirDrop channel
it holds.
