# filin RX diagnosis: AWDL action frames rejected after radiotap fix

## Finding

The most likely root cause is **filin does not strip the trailing 802.11 FCS when
radiotap flags say FCS is present**.

This is a direct divergence from owl:

- `owl/src/rx.c:512` strips the radiotap header.
- `owl/src/rx.c:514` calls `check_fcs(frame, flags)`.
- `owl/src/rx.c:456-463` rejects `BADFCS` and, when `IEEE80211_RADIOTAP_F_FCS`
  is set, removes the final 4 bytes with `BUF_TAKE(frame, 4)`.
- Only after that does owl read the fixed 24-byte `struct ieee80211_hdr` and
  dispatch action/data by frame control at `owl/src/rx.c:517-535`.

filin currently:

- Parses radiotap at `crates/filin-rs/src/lib.rs:1173`.
- Passes `radiotap.payload` directly into `ieee80211::parse_mac_header` at
  `crates/filin-rs/src/lib.rs:1174`.
- Does the same direct unstripped parse in the data fallback at
  `crates/filin-rs/src/lib.rs:1487-1491`.
- Never checks radiotap `BADFCS` and never strips a trailing FCS when radiotap
  `flags & 0x10 != 0`.

The real carl9170 radiotap header supplied earlier has `Flags=0x10`, i.e. FCS is
present:

```text
00 00 12 00 2e 48 00 00 10 ...
                              ^^ radiotap flags: IEEE80211_RADIOTAP_F_FCS
```

So filin is feeding the 4-byte FCS into the 802.11/AWDL payload parsers.

## Why the live error shape matches this

The live log says:

```text
err=Awdl(TooShort) data_err=Awdl(UnexpectedValue)
ieee80211_preview=d0 00 00 00 ff ff ff ff ff ff 4e 85 07 04 69 32
```

The preview starts with `0xd0`, so the 802.11 frame control is management/action.
The fixed 24-byte management header shape matches owl's `struct ieee80211_hdr`
in `owl/src/ieee80211.h:139-146`, and filin's fixed management-header split at
`crates/filin-rs/src/lib.rs:220-233` is not the primary divergence for this
specific `d0` frame.

Important: `err=Awdl(TooShort)` does **not** prove that
`awdl::parse_action_frame` rejected `body.len() < 16` at
`crates/filin-rs/src/lib.rs:400-403`.

In `rx::parse_action_frame`, filin maps two different AWDL parser failures into
the same outer variant:

- action header parse: `crates/filin-rs/src/lib.rs:1175`
- TLV parse: `crates/filin-rs/src/lib.rs:1176`

If the action header is valid but the trailing FCS remains after the real TLVs,
`awdl::parse_tlvs` will eventually treat the FCS bytes as another TLV header and
return `TooShort`. That is exactly consistent with `err=Awdl(TooShort)`.

The simultaneous `data_err=Awdl(UnexpectedValue)` is also consistent: the data
fallback is trying to parse the same management/action frame as an AWDL data
frame. Its body starts with vendor action category `0x7f`, not LLC/SNAP
`aa aa 03 ...`, so `awdl::decapsulate_data_frame` returns `UnexpectedValue`.
That data-path error is expected noise for action frames.

## Precise fix

Implement owl's FCS handling immediately after radiotap parsing and before any
802.11 header/body slicing.

Recommended shape:

1. Add radiotap flag constants in filin:

```rust
const RADIOTAP_F_FCS: u8 = 0x10;
const RADIOTAP_F_BADFCS: u8 = 0x40;
```

2. Add a helper that returns the 802.11 bytes with FCS removed:

```rust
fn ieee80211_payload_without_fcs<'a>(
    header: &'a radiotap::RadiotapHeader<'a>,
) -> Result<&'a [u8], ParseError> {
    let flags = header.flags.unwrap_or(0);
    if flags & RADIOTAP_F_BADFCS != 0 {
        return Err(ParseError::RadiotapBadFcs);
    }
    if flags & RADIOTAP_F_FCS != 0 {
        return header
            .payload
            .get(..header.payload.len().saturating_sub(4))
            .ok_or(ParseError::RadiotapFcsTooShort);
    }
    Ok(header.payload)
}
```

Use checked subtraction rather than `saturating_sub` if introducing a distinct
short-frame error.

3. In `rx::parse_action_frame`:

- Current line: `crates/filin-rs/src/lib.rs:1174`
- Change from parsing `radiotap.payload` to parsing the FCS-stripped 802.11
  payload.

4. In runtime data fallback:

- Current lines: `crates/filin-rs/src/lib.rs:1487-1491`
- Change `decapsulate_rx_data` to use the same FCS-stripped 802.11 payload.

5. In the ignored-frame preview helper:

- Current lines: `crates/filin-rs/src/lib.rs:1494-1498`
- Prefer previewing the FCS-stripped 802.11 payload. If stripping fails, log the
  unstripped preview and the FCS error.

6. Add regression tests:

- A radiotap header with `flags=0x10`, 24-byte management/action header, valid
  16-byte AWDL action header, one valid TLV, and 4 trailing FCS bytes should parse
  successfully and produce exactly one TLV.
- The same frame without FCS stripping should reproduce the current
  `parse_tlvs`-style `TooShort`, demonstrating this was the live bug.
- A radiotap header with `flags=0x40` should be rejected/ignored before AWDL
  parsing.

## Secondary cleanup

Split the outer RX AWDL error so future logs distinguish:

```rust
ParseError::AwdlAction(awdl::ParseError)
ParseError::AwdlTlv(awdl::ParseError)
```

Right now `crates/filin-rs/src/lib.rs:1175-1176` collapses both into
`ParseError::Awdl`, which made the live `TooShort` look like an action-header
length failure even though the TLV parser is the more likely source.

## Not the primary issue for the shown `d0` frames

The fixed 24-byte management/action header length is consistent with owl for
this specific frame:

- owl's `struct ieee80211_hdr` is 24 bytes in `owl/src/ieee80211.h:139-146`.
- owl strips exactly that header before action dispatch at `owl/src/rx.c:530`.
- filin's `ieee80211::parse_mac_header` also uses 24 bytes at
  `crates/filin-rs/src/lib.rs:220-233`.

Dynamic header length is still needed for QoS data, A-MSDU, addr4, and HT-control
coverage, but it does not explain the shown broadcast management/action preview
as well as the missing FCS strip does.
