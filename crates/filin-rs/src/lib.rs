pub mod radiotap {
    #[derive(Debug, PartialEq, Eq)]
    pub struct RadiotapHeader<'a> {
        pub header_len: usize,
        pub payload: &'a [u8],
        pub tsft: Option<u64>,
        pub flags: Option<u8>,
        pub antenna_signal_dbm: Option<i8>,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum RadiotapError {
        TooShort,
        UnsupportedVersion(u8),
        InvalidLength { header_len: usize, frame_len: usize },
        ExtendedPresentBitmap,
        UnsupportedField(u8),
    }

    pub fn parse_header(frame: &[u8]) -> Result<RadiotapHeader<'_>, RadiotapError> {
        if frame.len() < 8 {
            return Err(RadiotapError::TooShort);
        }
        if frame[0] != 0 {
            return Err(RadiotapError::UnsupportedVersion(frame[0]));
        }

        let header_len = u16::from_le_bytes([frame[2], frame[3]]) as usize;
        if header_len < 8 || header_len > frame.len() {
            return Err(RadiotapError::InvalidLength {
                header_len,
                frame_len: frame.len(),
            });
        }

        let present = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
        if present & (1 << 31) != 0 {
            return Err(RadiotapError::ExtendedPresentBitmap);
        }

        let mut offset = 8;
        let mut tsft = None;
        let mut flags = None;
        let mut antenna_signal_dbm = None;

        for bit in 0..32 {
            if present & (1 << bit) == 0 {
                continue;
            }

            match bit {
                0 => {
                    offset = align(offset, 8);
                    let bytes = read_field::<8>(frame, header_len, offset)?;
                    tsft = Some(u64::from_le_bytes(bytes));
                    offset += 8;
                }
                1 => {
                    let bytes = read_field::<1>(frame, header_len, offset)?;
                    flags = Some(bytes[0]);
                    offset += 1;
                }
                5 => {
                    let bytes = read_field::<1>(frame, header_len, offset)?;
                    antenna_signal_dbm = Some(bytes[0] as i8);
                    offset += 1;
                }
                _ => return Err(RadiotapError::UnsupportedField(bit)),
            }
        }

        Ok(RadiotapHeader {
            header_len,
            payload: &frame[header_len..],
            tsft,
            flags,
            antenna_signal_dbm,
        })
    }

    fn align(offset: usize, alignment: usize) -> usize {
        debug_assert!(alignment.is_power_of_two());
        (offset + alignment - 1) & !(alignment - 1)
    }

    fn read_field<const N: usize>(
        frame: &[u8],
        header_len: usize,
        offset: usize,
    ) -> Result<[u8; N], RadiotapError> {
        let end = offset.checked_add(N).ok_or(RadiotapError::TooShort)?;
        let bytes = frame
            .get(offset..end)
            .filter(|_| end <= header_len)
            .ok_or(RadiotapError::TooShort)?;
        Ok(bytes.try_into().expect("slice length matches field length"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_tsft_flags_and_rssi_from_v0_header() {
            let frame = [
                0x00, 0x00, 0x12, 0x00, // version, pad, length
                0x23, 0x00, 0x00, 0x00, // present: TSFT, flags, dBm antenna signal
                0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // TSFT
                0x10, // flags: frame includes FCS
                0xd6, // antenna signal: -42 dBm
                0xde, 0xad, // payload begins after radiotap header
            ];

            let parsed = parse_header(&frame).expect("valid radiotap header");

            assert_eq!(parsed.header_len, 18);
            assert_eq!(parsed.payload, &[0xde, 0xad]);
            assert_eq!(parsed.tsft, Some(0x0102_0304_0506_0708));
            assert_eq!(parsed.flags, Some(0x10));
            assert_eq!(parsed.antenna_signal_dbm, Some(-42));
        }
    }
}

pub mod ieee80211 {
    #[derive(Debug, PartialEq, Eq)]
    pub struct MacHeader<'a> {
        pub frame_control: u16,
        pub duration_id: u16,
        pub destination: [u8; 6],
        pub source: [u8; 6],
        pub bssid: [u8; 6],
        pub sequence_control: u16,
        pub body: &'a [u8],
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum ParseError {
        TooShort,
    }

    pub fn parse_mac_header(frame: &[u8]) -> Result<MacHeader<'_>, ParseError> {
        if frame.len() < 24 {
            return Err(ParseError::TooShort);
        }

        Ok(MacHeader {
            frame_control: u16::from_le_bytes([frame[0], frame[1]]),
            duration_id: u16::from_le_bytes([frame[2], frame[3]]),
            destination: read_addr(frame, 4)?,
            source: read_addr(frame, 10)?,
            bssid: read_addr(frame, 16)?,
            sequence_control: u16::from_le_bytes([frame[22], frame[23]]),
            body: &frame[24..],
        })
    }

    fn read_addr(frame: &[u8], offset: usize) -> Result<[u8; 6], ParseError> {
        Ok(frame
            .get(offset..offset + 6)
            .ok_or(ParseError::TooShort)?
            .try_into()
            .expect("slice length matches mac address length"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_three_address_management_action_header() {
            let frame = [
                0xd0, 0x00, // management action
                0x00, 0x00, // duration
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // destination
                0x02, 0x11, 0x22, 0x33, 0x44, 0x55, // source
                0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, // bssid
                0x10, 0x00, // sequence control
                0x7f, 0x50, // body
            ];

            let parsed = parse_mac_header(&frame).expect("valid 802.11 header");

            assert_eq!(parsed.frame_control, 0x00d0);
            assert_eq!(parsed.duration_id, 0);
            assert_eq!(parsed.destination, [0xff; 6]);
            assert_eq!(parsed.source, [0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
            assert_eq!(parsed.bssid, [0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb]);
            assert_eq!(parsed.sequence_control, 0x0010);
            assert_eq!(parsed.body, &[0x7f, 0x50]);
        }
    }
}

pub mod awdl {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ActionSubtype {
        Psf,
        Mif,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct ActionFrame<'a> {
        pub subtype: ActionSubtype,
        pub phy_tx: u32,
        pub target_tx: u32,
        pub tlvs: &'a [u8],
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum ParseError {
        TooShort,
        InvalidVendorHeader,
        UnsupportedSubtype(u8),
        UnexpectedValue,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct Tlv<'a> {
        pub kind: u8,
        pub value: &'a [u8],
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct SyncParameters {
        pub time_to_next_aw_tu: u16,
        pub aw_counter: u16,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct ElectionParametersV2 {
        pub master_addr: [u8; 6],
        pub sync_addr: [u8; 6],
        pub master_counter: u32,
        pub distance_to_master: u32,
        pub master_metric: u32,
        pub self_metric: u32,
        pub self_counter: u32,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct ChannelSequence {
        pub encoding: u8,
        pub channels: Vec<u16>,
    }

    pub fn parse_action_frame(body: &[u8]) -> Result<ActionFrame<'_>, ParseError> {
        if body.len() < 16 {
            return Err(ParseError::TooShort);
        }

        let valid = body[0] == 127
            && body[1..4] == [0x00, 0x17, 0xf2]
            && body[4] == 0x08
            && body[5] == 0x10;
        if !valid {
            return Err(ParseError::InvalidVendorHeader);
        }

        let subtype = match body[6] {
            0 => ActionSubtype::Psf,
            3 => ActionSubtype::Mif,
            other => return Err(ParseError::UnsupportedSubtype(other)),
        };

        Ok(ActionFrame {
            subtype,
            phy_tx: u32::from_le_bytes([body[8], body[9], body[10], body[11]]),
            target_tx: u32::from_le_bytes([body[12], body[13], body[14], body[15]]),
            tlvs: &body[16..],
        })
    }

    pub fn parse_tlvs(bytes: &[u8]) -> Result<Vec<Tlv<'_>>, ParseError> {
        let mut offset = 0;
        let mut tlvs = Vec::new();

        while offset < bytes.len() {
            let header = bytes.get(offset..offset + 3).ok_or(ParseError::TooShort)?;
            let kind = header[0];
            let len = u16::from_le_bytes([header[1], header[2]]) as usize;
            offset += 3;

            let end = offset.checked_add(len).ok_or(ParseError::TooShort)?;
            let value = bytes.get(offset..end).ok_or(ParseError::TooShort)?;
            tlvs.push(Tlv { kind, value });
            offset = end;
        }

        Ok(tlvs)
    }

    pub fn parse_sync_parameters(value: &[u8]) -> Result<SyncParameters, ParseError> {
        let time_to_next_aw_tu = read_le16(value, 1)?;
        let aw_counter = read_le16(value, 29)?;

        Ok(SyncParameters {
            time_to_next_aw_tu,
            aw_counter,
        })
    }

    pub fn parse_election_parameters_v2(
        value: &[u8],
    ) -> Result<ElectionParametersV2, ParseError> {
        Ok(ElectionParametersV2 {
            master_addr: read_addr(value, 0)?,
            sync_addr: read_addr(value, 6)?,
            master_counter: read_le32(value, 12)?,
            distance_to_master: read_le32(value, 16)?,
            master_metric: read_le32(value, 20)?,
            self_metric: read_le32(value, 24)?,
            self_counter: read_le32(value, 36)?,
        })
    }

    pub fn parse_channel_sequence(value: &[u8]) -> Result<ChannelSequence, ParseError> {
        let count = *value.first().ok_or(ParseError::TooShort)?;
        let encoding = *value.get(1).ok_or(ParseError::TooShort)?;
        let duplicate_count = *value.get(2).ok_or(ParseError::TooShort)?;
        let step_count = *value.get(3).ok_or(ParseError::TooShort)?;
        let fill_channel = read_le16(value, 4)?;

        if count != 15 || duplicate_count != 0 || step_count != 3 || fill_channel != 0xffff {
            return Err(ParseError::UnexpectedValue);
        }

        let field_len = match encoding {
            1 => 1,
            2 | 3 => 2,
            _ => return Err(ParseError::UnexpectedValue),
        };
        let mut offset = 6;
        let mut channels = Vec::with_capacity(16);
        for _ in 0..16 {
            let channel = match field_len {
                1 => *value.get(offset).ok_or(ParseError::TooShort)? as u16,
                2 => read_le16(value, offset)?,
                _ => unreachable!("field length is validated above"),
            };
            channels.push(channel);
            offset += field_len;
        }

        Ok(ChannelSequence { encoding, channels })
    }

    fn read_le16(bytes: &[u8], offset: usize) -> Result<u16, ParseError> {
        let field = bytes.get(offset..offset + 2).ok_or(ParseError::TooShort)?;
        Ok(u16::from_le_bytes([field[0], field[1]]))
    }

    fn read_le32(bytes: &[u8], offset: usize) -> Result<u32, ParseError> {
        let field = bytes.get(offset..offset + 4).ok_or(ParseError::TooShort)?;
        Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
    }

    fn read_addr(bytes: &[u8], offset: usize) -> Result<[u8; 6], ParseError> {
        Ok(bytes
            .get(offset..offset + 6)
            .ok_or(ParseError::TooShort)?
            .try_into()
            .expect("slice length matches mac address length"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_psf_vendor_action_header() {
            let body = [
                0x7f, // vendor-specific action category
                0x00, 0x17, 0xf2, // Apple OUI
                0x08, // AWDL type
                0x10, // AWDL version 1.0
                0x00, // PSF subtype
                0x00, // reserved
                0x44, 0x33, 0x22, 0x11, // phy_tx
                0x88, 0x77, 0x66, 0x55, // target_tx
                0x04, 0x02, 0x00, 0xaa, 0xbb, // first TLV
            ];

            let parsed = parse_action_frame(&body).expect("valid AWDL PSF action frame");

            assert_eq!(parsed.subtype, ActionSubtype::Psf);
            assert_eq!(parsed.phy_tx, 0x1122_3344);
            assert_eq!(parsed.target_tx, 0x5566_7788);
            assert_eq!(parsed.tlvs, &[0x04, 0x02, 0x00, 0xaa, 0xbb]);
        }

        #[test]
        fn parses_repeated_tlvs() {
            let tlvs = [
                0x04, 0x02, 0x00, 0xaa, 0xbb, // sync params fragment
                0x18, 0x04, 0x00, 0x01, 0x02, 0x03, 0x04, // election v2 fragment
            ];

            let parsed = parse_tlvs(&tlvs).expect("valid AWDL TLVs");

            assert_eq!(
                parsed,
                vec![
                    Tlv {
                        kind: 0x04,
                        value: &[0xaa, 0xbb]
                    },
                    Tlv {
                        kind: 0x18,
                        value: &[0x01, 0x02, 0x03, 0x04]
                    }
                ]
            );
        }

        #[test]
        fn parses_sync_parameters_tlv_timing_fields() {
            let mut value = [0u8; 31];
            value[1..3].copy_from_slice(&0x1234u16.to_le_bytes());
            value[29..31].copy_from_slice(&0x5678u16.to_le_bytes());

            let parsed = parse_sync_parameters(&value).expect("valid sync parameters TLV");

            assert_eq!(
                parsed,
                SyncParameters {
                    time_to_next_aw_tu: 0x1234,
                    aw_counter: 0x5678
                }
            );
        }

        #[test]
        fn parses_election_parameters_v2_tlv() {
            let mut value = [0u8; 40];
            value[0..6].copy_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            value[6..12].copy_from_slice(&[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
            value[12..16].copy_from_slice(&10u32.to_le_bytes());
            value[16..20].copy_from_slice(&2u32.to_le_bytes());
            value[20..24].copy_from_slice(&900u32.to_le_bytes());
            value[24..28].copy_from_slice(&800u32.to_le_bytes());
            value[36..40].copy_from_slice(&7u32.to_le_bytes());

            let parsed =
                parse_election_parameters_v2(&value).expect("valid election parameters v2 TLV");

            assert_eq!(
                parsed,
                ElectionParametersV2 {
                    master_addr: [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                    sync_addr: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55],
                    master_counter: 10,
                    distance_to_master: 2,
                    master_metric: 900,
                    self_metric: 800,
                    self_counter: 7,
                }
            );
        }

        #[test]
        fn parses_opclass_channel_sequence_tlv() {
            let mut value = vec![15, 2, 0, 3, 0xff, 0xff];
            for channel in 1u16..=16 {
                value.extend_from_slice(&channel.to_le_bytes());
            }

            let parsed = parse_channel_sequence(&value).expect("valid channel sequence TLV");

            assert_eq!(
                parsed,
                ChannelSequence {
                    encoding: 2,
                    channels: (1u16..=16).collect()
                }
            );
        }
    }
}

pub mod election {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ElectionState {
        pub self_addr: [u8; 6],
        pub sync_addr: [u8; 6],
        pub master_addr: [u8; 6],
        pub master_counter: u32,
        pub master_metric: u32,
        pub self_metric: u32,
        pub height: u32,
    }

    pub fn choose_sync_master(
        local: &ElectionState,
        peers: &[ElectionState],
    ) -> ElectionState {
        let mut selected = local.clone();

        for peer in peers {
            if should_adopt(peer, &selected) {
                selected = ElectionState {
                    self_addr: local.self_addr,
                    sync_addr: peer.self_addr,
                    master_addr: peer.master_addr,
                    master_counter: peer.master_counter,
                    master_metric: peer.master_metric,
                    self_metric: local.self_metric,
                    height: peer.height + 1,
                };
            }
        }

        selected
    }

    fn should_adopt(peer: &ElectionState, current: &ElectionState) -> bool {
        match compare_master(peer, current) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => {
                peer.height < current.height
                    || (peer.height == current.height && peer.self_addr > current.self_addr)
            }
        }
    }

    fn compare_master(a: &ElectionState, b: &ElectionState) -> std::cmp::Ordering {
        a.master_counter
            .cmp(&b.master_counter)
            .then_with(|| a.master_metric.cmp(&b.master_metric))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn state(addr: [u8; 6], master_metric: u32, height: u32) -> ElectionState {
            ElectionState {
                self_addr: addr,
                sync_addr: addr,
                master_addr: addr,
                master_counter: 1,
                master_metric,
                self_metric: master_metric,
                height,
            }
        }

        #[test]
        fn adopts_peer_with_higher_master_metric() {
            let local = state([0x02, 0, 0, 0, 0, 1], 100, 0);
            let peer = state([0x02, 0, 0, 0, 0, 2], 200, 0);

            let selected = choose_sync_master(&local, &[peer.clone()]);

            assert_eq!(selected.master_addr, peer.master_addr);
            assert_eq!(selected.sync_addr, peer.self_addr);
            assert_eq!(selected.master_metric, 200);
            assert_eq!(selected.height, 1);
        }

        #[test]
        fn breaks_equal_metric_tie_by_peer_address() {
            let local = state([0x02, 0, 0, 0, 0, 1], 100, 0);
            let lower = state([0x02, 0, 0, 0, 0, 2], 100, 0);
            let higher = state([0x02, 0, 0, 0, 0, 3], 100, 0);

            let selected = choose_sync_master(&local, &[lower, higher.clone()]);

            assert_eq!(selected.sync_addr, higher.self_addr);
        }
    }
}
