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

    pub fn build_management_action_frame(
        destination: [u8; 6],
        source: [u8; 6],
        bssid: [u8; 6],
        sequence_control: u16,
        body: &[u8],
    ) -> Vec<u8> {
        let mut frame = Vec::with_capacity(24 + body.len());
        frame.extend_from_slice(&0x00d0u16.to_le_bytes());
        frame.extend_from_slice(&0u16.to_le_bytes());
        frame.extend_from_slice(&destination);
        frame.extend_from_slice(&source);
        frame.extend_from_slice(&bssid);
        frame.extend_from_slice(&sequence_control.to_le_bytes());
        frame.extend_from_slice(body);
        frame
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

        #[test]
        fn builds_three_address_management_action_frame() {
            let frame = build_management_action_frame(
                [0xff; 6],
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                [0x00, 0x25, 0x00, 0xff, 0x94, 0x73],
                0x0010,
                &[0x7f, 0x00],
            );

            assert_eq!(&frame[0..2], &[0xd0, 0x00]);
            assert_eq!(&frame[4..10], &[0xff; 6]);
            assert_eq!(&frame[10..16], &[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            assert_eq!(&frame[16..22], &[0x00, 0x25, 0x00, 0xff, 0x94, 0x73]);
            assert_eq!(&frame[22..24], &[0x10, 0x00]);
            assert_eq!(&frame[24..], &[0x7f, 0x00]);
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

    #[derive(Debug, PartialEq, Eq)]
    pub struct EthernetFrame {
        pub bytes: Vec<u8>,
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

    pub fn decapsulate_data_frame(
        body: &[u8],
        source: [u8; 6],
        destination: [u8; 6],
    ) -> Result<EthernetFrame, ParseError> {
        if body.len() < 16 {
            return Err(ParseError::TooShort);
        }
        if body[0..3] != [0xaa, 0xaa, 0x03] || body[6..8] != [0x08, 0x00] {
            return Err(ParseError::UnexpectedValue);
        }

        let ethertype = [body[14], body[15]];
        let payload = &body[16..];
        let mut bytes = Vec::with_capacity(14 + payload.len());
        bytes.extend_from_slice(&destination);
        bytes.extend_from_slice(&source);
        bytes.extend_from_slice(&ethertype);
        bytes.extend_from_slice(payload);
        Ok(EthernetFrame { bytes })
    }

    pub fn build_action_body(
        subtype: ActionSubtype,
        phy_tx: u32,
        target_tx: u32,
        tlvs: &[u8],
    ) -> Vec<u8> {
        let subtype = match subtype {
            ActionSubtype::Psf => 0,
            ActionSubtype::Mif => 3,
        };
        let mut body = Vec::with_capacity(16 + tlvs.len());
        body.extend_from_slice(&[0x7f, 0x00, 0x17, 0xf2, 0x08, 0x10, subtype, 0x00]);
        body.extend_from_slice(&phy_tx.to_le_bytes());
        body.extend_from_slice(&target_tx.to_le_bytes());
        body.extend_from_slice(tlvs);
        body
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

        #[test]
        fn decapsulates_awdl_data_body_to_ethernet_frame() {
            let mut body = vec![
                0xaa, 0xaa, 0x03, 0x00, 0x17, 0xf2, 0x08, 0x00, // LLC/SNAP
                0x00, 0x00, // head
                0x01, 0x00, // seq
                0x00, 0x00, // pad
                0x86, 0xdd, // ethertype IPv6
            ];
            body.extend_from_slice(&[0x60, 0x00, 0x00, 0x00]);

            let frame = decapsulate_data_frame(
                &body,
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                [0x33, 0x33, 0x00, 0x00, 0x00, 0x01],
            )
            .expect("valid AWDL data frame");

            assert_eq!(
                frame.bytes,
                vec![
                    0x33, 0x33, 0x00, 0x00, 0x00, 0x01,
                    0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
                    0x86, 0xdd,
                    0x60, 0x00, 0x00, 0x00,
                ]
            );
        }

        #[test]
        fn builds_mif_vendor_action_body() {
            let body = build_action_body(ActionSubtype::Mif, 0x1122_3344, 0x5566_7788, &[0x04, 0, 0]);

            assert_eq!(
                body,
                vec![
                    0x7f, 0x00, 0x17, 0xf2, 0x08, 0x10, 0x03, 0x00,
                    0x44, 0x33, 0x22, 0x11,
                    0x88, 0x77, 0x66, 0x55,
                    0x04, 0x00, 0x00,
                ]
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

pub mod sync {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SyncState {
        pub last_update_us: u64,
        pub aw_counter: u16,
        pub aw_period_tu: u16,
        pub presence_mode: u16,
    }

    impl SyncState {
        pub fn new(now_us: u64) -> Self {
            Self {
                last_update_us: now_us,
                aw_counter: 0,
                aw_period_tu: 16,
                presence_mode: 4,
            }
        }

        pub fn update_from_master(
            &mut self,
            rx_hw_time_us: u64,
            time_to_next_aw_tu: u16,
            aw_counter: u16,
        ) {
            let eaw_period_tu = u64::from(self.presence_mode) * u64::from(self.aw_period_tu);
            self.last_update_us =
                rx_hw_time_us - tu_to_us(eaw_period_tu - u64::from(time_to_next_aw_tu));
            self.aw_counter = aw_counter & 0xfffc;
        }

        pub fn sync_error_tu(
            &self,
            now_us: u64,
            time_to_next_aw_tu: u16,
            aw_counter: u16,
        ) -> i64 {
            let master_delta = (u64::from(aw_counter) / u64::from(self.presence_mode)) as i64
                - i64::from(self.current_eaw(now_us));
            let aw_delta = i64::from(time_to_next_aw_tu) - i64::from(self.next_aw_tu(now_us));
            master_delta * i64::from(self.presence_mode) * i64::from(self.aw_period_tu) - aw_delta
        }

        pub fn next_aw_tu(&self, now_us: u64) -> u16 {
            let eaw_period_tu = u64::from(self.presence_mode) * u64::from(self.aw_period_tu);
            let time_since_tu = us_to_tu(now_us - self.last_update_us);
            (eaw_period_tu - (time_since_tu % eaw_period_tu)) as u16
        }

        pub fn current_aw(&self, now_us: u64) -> u16 {
            let eaw_period_tu = u64::from(self.presence_mode) * u64::from(self.aw_period_tu);
            let time_since_tu = us_to_tu(now_us - self.last_update_us);
            let current_aw = u64::from(self.aw_counter)
                + (time_since_tu % eaw_period_tu) / u64::from(self.aw_period_tu)
                + u64::from(self.presence_mode) * (time_since_tu / eaw_period_tu);
            current_aw as u16
        }

        pub fn current_eaw(&self, now_us: u64) -> u16 {
            self.current_aw(now_us) / self.presence_mode
        }
    }

    fn tu_to_us(tu: u64) -> u64 {
        tu * 1024
    }

    fn us_to_tu(us: u64) -> u64 {
        us / 1024
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TsftBridge {
        offset_us: Option<i64>,
        sanity_clamp_us: i64,
    }

    impl Default for TsftBridge {
        fn default() -> Self {
            Self {
                offset_us: None,
                sanity_clamp_us: 100_000,
            }
        }
    }

    impl TsftBridge {
        pub fn timestamp_rx(&mut self, hw_tsft_us: u64, host_now_us: u64) -> u64 {
            let instant_offset = host_now_us as i64 - hw_tsft_us as i64;
            let Some(offset) = self.offset_us else {
                self.offset_us = Some(instant_offset);
                return host_now_us;
            };

            let candidate = hw_tsft_us as i64 + offset;
            if (candidate - host_now_us as i64).abs() > self.sanity_clamp_us {
                self.offset_us = Some(instant_offset);
                return host_now_us;
            }

            let next_offset = offset + ((instant_offset - offset) >> 4);
            self.offset_us = Some(next_offset);
            (hw_tsft_us as i64 + next_offset) as u64
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn updates_schedule_from_hardware_timestamped_master_sync() {
            let mut sync = SyncState::new(1_000_000);

            sync.update_from_master(2_000_000, 20, 18);

            assert_eq!(sync.last_update_us, 2_000_000 - (64 - 20) * 1024);
            assert_eq!(sync.aw_counter, 16);
        }

        #[test]
        fn bridges_tsft_to_monotonic_with_ema_offset() {
            let mut bridge = TsftBridge::default();

            assert_eq!(bridge.timestamp_rx(1_000, 101_000), 101_000);
            assert_eq!(bridge.timestamp_rx(2_000, 104_000), 102_125);
        }

        #[test]
        fn computes_zero_sync_error_for_matching_master_schedule() {
            let mut sync = SyncState::new(0);
            sync.update_from_master(2_000_000, 20, 18);

            assert_eq!(sync.sync_error_tu(2_000_000, 20, 18), 0);
        }
    }
}

pub mod channel {
    pub fn channel_to_frequency_mhz(channel: u16) -> Option<u16> {
        match channel {
            0 => None,
            14 => Some(2484),
            1..=13 => Some(2407 + channel * 5),
            32..=181 => Some(5000 + channel * 5),
            182..=196 => Some(4000 + channel * 5),
            _ => None,
        }
    }

    pub fn channel_for_eaw(sequence: &[u16], current_eaw: u16) -> Option<u16> {
        if sequence.is_empty() {
            return None;
        }
        sequence
            .get(usize::from(current_eaw) % sequence.len())
            .copied()
            .filter(|channel| *channel != 0)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn converts_common_awdl_channels_to_frequency() {
            assert_eq!(channel_to_frequency_mhz(6), Some(2437));
            assert_eq!(channel_to_frequency_mhz(44), Some(5220));
            assert_eq!(channel_to_frequency_mhz(149), Some(5745));
        }

        #[test]
        fn selects_channel_by_eaw_slot() {
            let sequence = [44, 44, 149, 6, 0, 0, 44, 149, 6, 44, 0, 0, 149, 44, 6, 44];

            assert_eq!(channel_for_eaw(&sequence, 0), Some(44));
            assert_eq!(channel_for_eaw(&sequence, 2), Some(149));
            assert_eq!(channel_for_eaw(&sequence, 17), Some(44));
        }
    }
}

pub mod tx {
    use std::collections::VecDeque;

    #[derive(Debug, PartialEq, Eq)]
    pub enum InjectError {
        WouldBlock,
        Failed,
    }

    pub trait Injector {
        fn inject(&mut self, frame: &[u8]) -> Result<(), InjectError>;
    }

    #[derive(Debug, Default)]
    pub struct TxQueue {
        pending: VecDeque<Vec<u8>>,
    }

    impl TxQueue {
        pub fn push(&mut self, frame: Vec<u8>) {
            self.pending.push_back(frame);
        }

        pub fn pending_len(&self) -> usize {
            self.pending.len()
        }

        pub fn flush<I: Injector>(&mut self, injector: &mut I) -> Result<usize, InjectError> {
            let mut sent = 0;
            while let Some(frame) = self.pending.pop_front() {
                match injector.inject(&frame) {
                    Ok(()) => sent += 1,
                    Err(InjectError::WouldBlock) => {
                        self.pending.push_front(frame);
                        return Ok(sent);
                    }
                    Err(InjectError::Failed) => {
                        self.pending.push_front(frame);
                        return Err(InjectError::Failed);
                    }
                }
            }
            Ok(sent)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        struct BlockingInjector {
            calls: usize,
        }

        impl Injector for BlockingInjector {
            fn inject(&mut self, _frame: &[u8]) -> Result<(), InjectError> {
                self.calls += 1;
                Err(InjectError::WouldBlock)
            }
        }

        #[test]
        fn keeps_frame_queued_when_inject_would_block() {
            let mut queue = TxQueue::default();
            queue.push(vec![1, 2, 3]);
            let mut injector = BlockingInjector { calls: 0 };

            assert_eq!(queue.flush(&mut injector), Ok(0));
            assert_eq!(injector.calls, 1);
            assert_eq!(queue.pending_len(), 1);
        }
    }
}

pub mod iface {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Config {
        pub monitor_iface: String,
        pub host_iface: String,
        pub anchor_channel: u16,
        pub assume_monitor: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum ConfigError {
        MissingMonitorInterface,
        UnsupportedAnchorChannel(u16),
    }

    pub fn validate_config(config: Config) -> Result<Config, ConfigError> {
        if config.monitor_iface.is_empty() {
            return Err(ConfigError::MissingMonitorInterface);
        }
        match config.anchor_channel {
            6 | 44 | 149 => Ok(config),
            other => Err(ConfigError::UnsupportedAnchorChannel(other)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn accepts_default_awdl0_channel_44_config() {
            let config = Config {
                monitor_iface: "wlan0mon".into(),
                host_iface: "awdl0".into(),
                anchor_channel: 44,
                assume_monitor: true,
            };

            assert_eq!(validate_config(config.clone()), Ok(config));
        }
    }
}

pub mod rx {
    use crate::{awdl, ieee80211, radiotap, sync::TsftBridge};

    #[derive(Debug, PartialEq, Eq)]
    pub struct ParsedActionFrame<'a> {
        pub rx_time_us: u64,
        pub rssi_dbm: Option<i8>,
        pub source: [u8; 6],
        pub destination: [u8; 6],
        pub action: awdl::ActionFrame<'a>,
        pub tlvs: Vec<awdl::Tlv<'a>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum ParseError {
        Radiotap,
        Ieee80211,
        Awdl,
        MissingTsft,
    }

    pub fn parse_action_frame<'a>(
        frame: &'a [u8],
        host_now_us: u64,
        bridge: &mut TsftBridge,
    ) -> Result<ParsedActionFrame<'a>, ParseError> {
        let radiotap = radiotap::parse_header(frame).map_err(|_| ParseError::Radiotap)?;
        let hw_tsft = radiotap.tsft.ok_or(ParseError::MissingTsft)?;
        let mac = ieee80211::parse_mac_header(radiotap.payload).map_err(|_| ParseError::Ieee80211)?;
        let action = awdl::parse_action_frame(mac.body).map_err(|_| ParseError::Awdl)?;
        let tlvs = awdl::parse_tlvs(action.tlvs).map_err(|_| ParseError::Awdl)?;
        let rx_time_us = bridge.timestamp_rx(hw_tsft, host_now_us);

        Ok(ParsedActionFrame {
            rx_time_us,
            rssi_dbm: radiotap.antenna_signal_dbm,
            source: mac.source,
            destination: mac.destination,
            action,
            tlvs,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_radiotap_80211_awdl_action_frame_with_tsft() {
            let mut frame = vec![
                0x00, 0x00, 0x12, 0x00, // radiotap version, pad, len
                0x23, 0x00, 0x00, 0x00, // TSFT, flags, RSSI
                0xe8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // TSFT 1000 us
                0x00, // flags
                0xd8, // -40 dBm
            ];
            frame.extend_from_slice(&[
                0xd0, 0x00, 0x00, 0x00, // action header
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // destination
                0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, // source
                0x00, 0x25, 0x00, 0xff, 0x94, 0x73, // bssid
                0x00, 0x00, // sequence control
                0x7f, 0x00, 0x17, 0xf2, 0x08, 0x10, 0x03, 0x00, // AWDL MIF
                0x00, 0x00, 0x00, 0x00, // phy_tx
                0x00, 0x00, 0x00, 0x00, // target_tx
                0x04, 0x02, 0x00, 0xaa, 0xbb, // TLV
            ]);

            let mut bridge = TsftBridge::default();
            let parsed = parse_action_frame(&frame, 101_000, &mut bridge)
                .expect("valid RX AWDL action frame");

            assert_eq!(parsed.rx_time_us, 101_000);
            assert_eq!(parsed.rssi_dbm, Some(-40));
            assert_eq!(parsed.source, [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            assert_eq!(parsed.destination, [0xff; 6]);
            assert_eq!(parsed.action.subtype, awdl::ActionSubtype::Mif);
            assert_eq!(parsed.tlvs[0].kind, 0x04);
        }
    }
}

pub mod cli {
    use crate::iface;
    use clap::Parser;

    #[derive(Debug, PartialEq, Eq)]
    pub enum CliError {
        Invalid,
    }

    #[derive(Debug, Parser)]
    #[command(name = "filin", disable_help_flag = true)]
    struct Args {
        #[arg(short = 'i')]
        monitor_iface: String,
        #[arg(short = 'c', default_value_t = 44)]
        anchor_channel: u16,
        #[arg(short = 'h', default_value = "awdl0")]
        host_iface: String,
        #[arg(short = 'N')]
        assume_monitor: bool,
    }

    pub fn parse_config_from<I, S>(args: I) -> Result<iface::Config, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString> + Clone,
    {
        let args = Args::try_parse_from(args).map_err(|_| CliError::Invalid)?;
        iface::validate_config(iface::Config {
            monitor_iface: args.monitor_iface,
            host_iface: args.host_iface,
            anchor_channel: args.anchor_channel,
            assume_monitor: args.assume_monitor,
        })
        .map_err(|_| CliError::Invalid)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_owl_compatible_interface_flags() {
            let config = parse_config_from(["filin", "-i", "wlan0mon", "-c", "44", "-h", "awdl0", "-N"])
                .expect("valid CLI args");

            assert_eq!(
                config,
                iface::Config {
                    monitor_iface: "wlan0mon".into(),
                    host_iface: "awdl0".into(),
                    anchor_channel: 44,
                    assume_monitor: true,
                }
            );
        }
    }
}

pub mod runtime {
    use std::fs::File;

    use crate::{iface::Config, os};

    #[derive(Debug, PartialEq, Eq)]
    pub enum Error {
        Tap,
        Netdev,
        Packet,
    }

    pub struct Links {
        pub tap: File,
        pub packet: os::packet::PacketSocket,
    }

    pub fn open_links(config: &Config) -> Result<Links, Error> {
        let tap = os::tun::open_tap(&config.host_iface).map_err(|_| Error::Tap)?;
        let monitor_addr = os::netdev::get_hwaddr(&config.monitor_iface).map_err(|_| Error::Netdev)?;
        os::netdev::set_hwaddr(&config.host_iface, monitor_addr).map_err(|_| Error::Netdev)?;
        os::netdev::set_mtu(&config.host_iface, 1450).map_err(|_| Error::Netdev)?;
        os::netdev::set_up(&config.host_iface).map_err(|_| Error::Netdev)?;
        let packet =
            os::packet::PacketSocket::open_bound(&config.monitor_iface).map_err(|_| Error::Packet)?;

        Ok(Links { tap, packet })
    }
}

pub mod os {
    pub mod tun {
        use std::fs::{File, OpenOptions};
        use std::os::fd::AsRawFd;

        #[derive(Debug, PartialEq, Eq)]
        pub enum Error {
            InvalidName,
            Io(i32),
        }

        #[derive(Debug, PartialEq, Eq)]
        pub struct IfReqPlan {
            pub name: [u8; libc::IFNAMSIZ],
            pub flags: libc::c_short,
        }

        pub fn plan_tap_ifreq(name: &str) -> Result<IfReqPlan, Error> {
            let bytes = name.as_bytes();
            if bytes.is_empty() || bytes.len() >= libc::IFNAMSIZ {
                return Err(Error::InvalidName);
            }

            let mut if_name = [0u8; libc::IFNAMSIZ];
            if_name[..bytes.len()].copy_from_slice(bytes);
            Ok(IfReqPlan {
                name: if_name,
                flags: (libc::IFF_TAP | libc::IFF_NO_PI) as libc::c_short,
            })
        }

        pub fn open_tap(name: &str) -> Result<File, Error> {
            let plan = plan_tap_ifreq(name)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/net/tun")
                .map_err(errno_from_io)?;
            let mut request = TunIfReq::from_plan(&plan);

            // SAFETY: request is a C-compatible ifreq prefix accepted by TUNSETIFF;
            // the fd is an open /dev/net/tun file and the kernel writes only within it.
            let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &mut request) };
            if rc < 0 {
                return Err(last_errno());
            }

            Ok(file)
        }

        #[repr(C)]
        struct TunIfReq {
            name: [libc::c_char; libc::IFNAMSIZ],
            flags: libc::c_short,
        }

        impl TunIfReq {
            fn from_plan(plan: &IfReqPlan) -> Self {
                let mut name = [0; libc::IFNAMSIZ];
                for (dst, src) in name.iter_mut().zip(plan.name) {
                    *dst = src as libc::c_char;
                }
                Self {
                    name,
                    flags: plan.flags,
                }
            }
        }

        fn errno_from_io(err: std::io::Error) -> Error {
            Error::Io(err.raw_os_error().unwrap_or(libc::EIO))
        }

        fn last_errno() -> Error {
            Error::Io(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_tap_ifreq_for_awdl0_without_packet_info() {
                let plan = plan_tap_ifreq("awdl0").expect("valid tap name");

                assert_eq!(&plan.name[..5], b"awdl0");
                assert_eq!(plan.name[5], 0);
                assert_eq!(plan.flags, (libc::IFF_TAP | libc::IFF_NO_PI) as libc::c_short);
            }
        }
    }

    pub mod packet {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

        #[derive(Debug, PartialEq, Eq)]
        pub struct BindPlan {
            pub ifindex: libc::c_int,
            pub protocol_be: u16,
        }

        pub fn plan_bind(ifindex: libc::c_int) -> Result<BindPlan, Error> {
            if ifindex <= 0 {
                return Err(Error::InvalidIfIndex);
            }
            Ok(BindPlan {
                ifindex,
                protocol_be: (libc::ETH_P_ALL as u16).to_be(),
            })
        }

        #[derive(Debug, PartialEq, Eq)]
        pub enum Error {
            InvalidIfIndex,
            InvalidName,
            Io(i32),
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_packet_bind_for_eth_p_all() {
                let plan = plan_bind(7).expect("valid ifindex");

                assert_eq!(plan.ifindex, 7);
                assert_eq!(plan.protocol_be, (libc::ETH_P_ALL as u16).to_be());
            }
        }

        pub struct PacketSocket {
            fd: OwnedFd,
        }

        impl PacketSocket {
            pub fn open_bound(iface: &str) -> Result<Self, Error> {
                let iface = CString::new(iface).map_err(|_| Error::InvalidName)?;
                // SAFETY: iface is a valid NUL-terminated C string.
                let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
                if ifindex == 0 {
                    return Err(last_errno());
                }
                Self::open_bound_ifindex(ifindex as libc::c_int)
            }

            pub fn open_bound_ifindex(ifindex: libc::c_int) -> Result<Self, Error> {
                let plan = plan_bind(ifindex)?;
                // SAFETY: socket arguments are constants for an AF_PACKET raw socket.
                let fd = unsafe {
                    libc::socket(
                        libc::AF_PACKET,
                        libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                        libc::ETH_P_ALL.to_be(),
                    )
                };
                if fd < 0 {
                    return Err(last_errno());
                }
                // SAFETY: fd was just returned by socket and is uniquely owned here.
                let fd = unsafe { OwnedFd::from_raw_fd(fd) };

                let addr = libc::sockaddr_ll {
                    sll_family: libc::AF_PACKET as libc::c_ushort,
                    sll_protocol: plan.protocol_be,
                    sll_ifindex: plan.ifindex,
                    sll_hatype: 0,
                    sll_pkttype: 0,
                    sll_halen: 0,
                    sll_addr: [0; 8],
                };
                // SAFETY: addr points to a valid sockaddr_ll for the duration of the call.
                let rc = unsafe {
                    libc::bind(
                        fd.as_raw_fd(),
                        (&addr as *const libc::sockaddr_ll).cast::<libc::sockaddr>(),
                        std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                    )
                };
                if rc < 0 {
                    return Err(last_errno());
                }

                Ok(Self { fd })
            }

            pub fn recv(&self, buf: &mut [u8]) -> Result<usize, Error> {
                // SAFETY: buf is valid writable memory for buf.len() bytes.
                let rc = unsafe {
                    libc::recv(
                        self.fd.as_raw_fd(),
                        buf.as_mut_ptr().cast::<libc::c_void>(),
                        buf.len(),
                        0,
                    )
                };
                if rc < 0 {
                    return Err(last_errno());
                }
                Ok(rc as usize)
            }

            pub fn send(&self, frame: &[u8]) -> Result<usize, Error> {
                // SAFETY: frame is valid readable memory for frame.len() bytes.
                let rc = unsafe {
                    libc::send(
                        self.fd.as_raw_fd(),
                        frame.as_ptr().cast::<libc::c_void>(),
                        frame.len(),
                        0,
                    )
                };
                if rc < 0 {
                    return Err(last_errno());
                }
                Ok(rc as usize)
            }

            pub fn as_raw_fd(&self) -> RawFd {
                self.fd.as_raw_fd()
            }
        }

        fn last_errno() -> Error {
            Error::Io(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        }
    }

    pub mod netdev {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

        #[derive(Debug, PartialEq, Eq)]
        pub enum Error {
            InvalidName,
            Io(i32),
        }

        #[derive(Debug, PartialEq, Eq)]
        pub struct IfName {
            pub bytes: [u8; libc::IFNAMSIZ],
        }

        pub fn plan_if_name(name: &str) -> Result<IfName, Error> {
            let raw = name.as_bytes();
            if raw.is_empty() || raw.len() >= libc::IFNAMSIZ {
                return Err(Error::InvalidName);
            }
            let mut bytes = [0u8; libc::IFNAMSIZ];
            bytes[..raw.len()].copy_from_slice(raw);
            Ok(IfName { bytes })
        }

        pub fn up_flags(existing: libc::c_short) -> libc::c_short {
            existing | (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short
        }

        pub fn get_hwaddr(iface: &str) -> Result<[u8; 6], Error> {
            let socket = control_socket()?;
            let mut request = IfReqHwAddr::new(iface)?;
            // SAFETY: request is a valid ifreq-compatible buffer for SIOCGIFHWADDR.
            let rc = unsafe {
                libc::ioctl(
                    socket.as_raw_fd(),
                    libc::SIOCGIFHWADDR,
                    &mut request as *mut IfReqHwAddr,
                )
            };
            if rc < 0 {
                return Err(last_errno());
            }
            Ok(request.hwaddr())
        }

        pub fn set_hwaddr(iface: &str, hwaddr: [u8; 6]) -> Result<(), Error> {
            let socket = control_socket()?;
            let mut request = IfReqHwAddr::new(iface)?;
            request.set_hwaddr(hwaddr);
            // SAFETY: request is a valid ifreq-compatible buffer for SIOCSIFHWADDR.
            let rc = unsafe {
                libc::ioctl(
                    socket.as_raw_fd(),
                    libc::SIOCSIFHWADDR,
                    &mut request as *mut IfReqHwAddr,
                )
            };
            if rc < 0 {
                return Err(last_errno());
            }
            Ok(())
        }

        pub fn set_mtu(iface: &str, mtu: libc::c_int) -> Result<(), Error> {
            let socket = control_socket()?;
            let mut request = IfReqMtu::new(iface, mtu)?;
            // SAFETY: request is a valid ifreq-compatible buffer for SIOCSIFMTU.
            let rc = unsafe {
                libc::ioctl(
                    socket.as_raw_fd(),
                    libc::SIOCSIFMTU,
                    &mut request as *mut IfReqMtu,
                )
            };
            if rc < 0 {
                return Err(last_errno());
            }
            Ok(())
        }

        pub fn set_up(iface: &str) -> Result<(), Error> {
            let socket = control_socket()?;
            let mut request = IfReqFlags::new(iface, 0)?;
            // SAFETY: request is a valid ifreq-compatible buffer for SIOCGIFFLAGS.
            let rc = unsafe {
                libc::ioctl(
                    socket.as_raw_fd(),
                    libc::SIOCGIFFLAGS,
                    &mut request as *mut IfReqFlags,
                )
            };
            if rc < 0 {
                return Err(last_errno());
            }
            request.flags = up_flags(request.flags);
            // SAFETY: request is a valid ifreq-compatible buffer for SIOCSIFFLAGS.
            let rc = unsafe {
                libc::ioctl(
                    socket.as_raw_fd(),
                    libc::SIOCSIFFLAGS,
                    &mut request as *mut IfReqFlags,
                )
            };
            if rc < 0 {
                return Err(last_errno());
            }
            Ok(())
        }

        fn control_socket() -> Result<OwnedFd, Error> {
            // SAFETY: socket arguments open a datagram IPv4 control socket for ioctls.
            let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
            if fd < 0 {
                return Err(last_errno());
            }
            // SAFETY: fd was just returned by socket and is uniquely owned here.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }

        #[repr(C)]
        struct IfReqFlags {
            name: [libc::c_char; libc::IFNAMSIZ],
            flags: libc::c_short,
        }

        #[repr(C)]
        struct IfReqMtu {
            name: [libc::c_char; libc::IFNAMSIZ],
            mtu: libc::c_int,
        }

        #[repr(C)]
        struct IfReqHwAddr {
            name: [libc::c_char; libc::IFNAMSIZ],
            addr: libc::sockaddr,
        }

        impl IfReqFlags {
            fn new(iface: &str, flags: libc::c_short) -> Result<Self, Error> {
                Ok(Self {
                    name: c_name(iface)?,
                    flags,
                })
            }
        }

        impl IfReqMtu {
            fn new(iface: &str, mtu: libc::c_int) -> Result<Self, Error> {
                Ok(Self {
                    name: c_name(iface)?,
                    mtu,
                })
            }
        }

        impl IfReqHwAddr {
            fn new(iface: &str) -> Result<Self, Error> {
                Ok(Self {
                    name: c_name(iface)?,
                    addr: libc::sockaddr {
                        sa_family: libc::ARPHRD_ETHER as libc::sa_family_t,
                        sa_data: [0; 14],
                    },
                })
            }

            fn set_hwaddr(&mut self, hwaddr: [u8; 6]) {
                self.addr.sa_family = libc::ARPHRD_ETHER as libc::sa_family_t;
                for (dst, src) in self.addr.sa_data.iter_mut().zip(hwaddr) {
                    *dst = src as libc::c_char;
                }
            }

            fn hwaddr(&self) -> [u8; 6] {
                let mut hwaddr = [0u8; 6];
                for (dst, src) in hwaddr.iter_mut().zip(self.addr.sa_data) {
                    *dst = src as u8;
                }
                hwaddr
            }
        }

        fn c_name(name: &str) -> Result<[libc::c_char; libc::IFNAMSIZ], Error> {
            let planned = plan_if_name(name)?;
            let mut out = [0; libc::IFNAMSIZ];
            for (dst, src) in out.iter_mut().zip(planned.bytes) {
                *dst = src as libc::c_char;
            }
            Ok(out)
        }

        fn last_errno() -> Error {
            Error::Io(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_if_name_and_up_flags() {
                let name = plan_if_name("awdl0").expect("valid if name");

                assert_eq!(&name.bytes[..5], b"awdl0");
                assert_eq!(name.bytes[5], 0);
                assert_eq!(up_flags(0), (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short);
            }
        }
    }

    pub mod nl80211 {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

        const GENL_ID_CTRL: u16 = 0x10;
        const CTRL_CMD_GETFAMILY: u8 = 3;
        const CTRL_ATTR_FAMILY_ID: u16 = 1;
        const CTRL_ATTR_FAMILY_NAME: u16 = 2;
        const NL80211_CMD_SET_CHANNEL: u8 = 65;
        const NL80211_ATTR_IFINDEX: u16 = 3;
        const NL80211_ATTR_WIPHY_FREQ: u16 = 38;

        #[derive(Debug, PartialEq, Eq)]
        pub struct ChannelRequest {
            pub ifindex: libc::c_int,
            pub frequency_mhz: u32,
        }

        pub fn plan_set_channel(ifindex: libc::c_int, frequency_mhz: u32) -> Result<ChannelRequest, Error> {
            if ifindex <= 0 || frequency_mhz == 0 {
                return Err(Error::Invalid);
            }
            Ok(ChannelRequest {
                ifindex,
                frequency_mhz,
            })
        }

        #[derive(Debug, PartialEq, Eq)]
        pub enum Error {
            Invalid,
            InvalidName,
            Io(i32),
            Protocol,
        }

        pub fn set_channel(iface: &str, frequency_mhz: u32) -> Result<(), Error> {
            let iface = CString::new(iface).map_err(|_| Error::InvalidName)?;
            // SAFETY: iface is a valid NUL-terminated C string.
            let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
            if ifindex == 0 {
                return Err(last_errno());
            }
            set_channel_ifindex(ifindex as libc::c_int, frequency_mhz)
        }

        pub fn set_channel_ifindex(ifindex: libc::c_int, frequency_mhz: u32) -> Result<(), Error> {
            let request = plan_set_channel(ifindex, frequency_mhz)?;
            let socket = NetlinkSocket::open()?;
            let family_id = socket.resolve_family("nl80211")?;
            let mut payload = vec![NL80211_CMD_SET_CHANNEL, 1, 0, 0];
            push_attr_u32(&mut payload, NL80211_ATTR_IFINDEX, request.ifindex as u32);
            push_attr_u32(&mut payload, NL80211_ATTR_WIPHY_FREQ, request.frequency_mhz);
            socket.request(family_id, libc::NLM_F_REQUEST as u16 | libc::NLM_F_ACK as u16, &payload)?;
            Ok(())
        }

        struct NetlinkSocket {
            fd: OwnedFd,
            seq: std::cell::Cell<u32>,
        }

        impl NetlinkSocket {
            fn open() -> Result<Self, Error> {
                // SAFETY: socket arguments open a generic netlink socket.
                let fd = unsafe {
                    libc::socket(
                        libc::AF_NETLINK,
                        libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                        libc::NETLINK_GENERIC,
                    )
                };
                if fd < 0 {
                    return Err(last_errno());
                }
                // SAFETY: fd was just returned by socket and is uniquely owned here.
                Ok(Self {
                    fd: unsafe { OwnedFd::from_raw_fd(fd) },
                    seq: std::cell::Cell::new(1),
                })
            }

            fn resolve_family(&self, family: &str) -> Result<u16, Error> {
                let mut payload = vec![CTRL_CMD_GETFAMILY, 1, 0, 0];
                push_attr_bytes(&mut payload, CTRL_ATTR_FAMILY_NAME, family.as_bytes(), true);
                let response = self.request(GENL_ID_CTRL, libc::NLM_F_REQUEST as u16, &payload)?;
                parse_family_id(&response).ok_or(Error::Protocol)
            }

            fn request(&self, nlmsg_type: u16, flags: u16, payload: &[u8]) -> Result<Vec<u8>, Error> {
                let seq = self.seq.get();
                self.seq.set(seq.wrapping_add(1));
                let mut msg = Vec::with_capacity(16 + payload.len());
                msg.extend_from_slice(&((16 + payload.len()) as u32).to_ne_bytes());
                msg.extend_from_slice(&nlmsg_type.to_ne_bytes());
                msg.extend_from_slice(&flags.to_ne_bytes());
                msg.extend_from_slice(&seq.to_ne_bytes());
                msg.extend_from_slice(&0u32.to_ne_bytes());
                msg.extend_from_slice(payload);

                // SAFETY: msg is valid readable memory for msg.len() bytes.
                let sent = unsafe {
                    libc::send(
                        self.fd.as_raw_fd(),
                        msg.as_ptr().cast::<libc::c_void>(),
                        msg.len(),
                        0,
                    )
                };
                if sent < 0 {
                    return Err(last_errno());
                }

                let mut buf = vec![0u8; 8192];
                // SAFETY: buf is valid writable memory for buf.len() bytes.
                let len = unsafe {
                    libc::recv(
                        self.fd.as_raw_fd(),
                        buf.as_mut_ptr().cast::<libc::c_void>(),
                        buf.len(),
                        0,
                    )
                };
                if len < 0 {
                    return Err(last_errno());
                }
                buf.truncate(len as usize);
                parse_ack_or_payload(&buf)
            }
        }

        fn parse_ack_or_payload(buf: &[u8]) -> Result<Vec<u8>, Error> {
            if buf.len() < 16 {
                return Err(Error::Protocol);
            }
            let len = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            let kind = u16::from_ne_bytes([buf[4], buf[5]]);
            if len > buf.len() || len < 16 {
                return Err(Error::Protocol);
            }
            if kind == libc::NLMSG_ERROR as u16 {
                if len < 20 {
                    return Err(Error::Protocol);
                }
                let code = i32::from_ne_bytes([buf[16], buf[17], buf[18], buf[19]]);
                return if code == 0 { Ok(Vec::new()) } else { Err(Error::Io(-code)) };
            }
            Ok(buf[16..len].to_vec())
        }

        fn parse_family_id(payload: &[u8]) -> Option<u16> {
            if payload.len() < 4 {
                return None;
            }
            let mut attrs = &payload[4..];
            while attrs.len() >= 4 {
                let len = u16::from_ne_bytes([attrs[0], attrs[1]]) as usize;
                let kind = u16::from_ne_bytes([attrs[2], attrs[3]]);
                if len < 4 || len > attrs.len() {
                    return None;
                }
                if kind == CTRL_ATTR_FAMILY_ID && len >= 6 {
                    return Some(u16::from_ne_bytes([attrs[4], attrs[5]]));
                }
                let aligned = align4(len);
                if aligned > attrs.len() {
                    return None;
                }
                attrs = &attrs[aligned..];
            }
            None
        }

        fn push_attr_u32(buf: &mut Vec<u8>, kind: u16, value: u32) {
            push_attr_bytes(buf, kind, &value.to_ne_bytes(), false);
        }

        fn push_attr_bytes(buf: &mut Vec<u8>, kind: u16, value: &[u8], nul: bool) {
            let payload_len = value.len() + usize::from(nul);
            let len = 4 + payload_len;
            buf.extend_from_slice(&(len as u16).to_ne_bytes());
            buf.extend_from_slice(&kind.to_ne_bytes());
            buf.extend_from_slice(value);
            if nul {
                buf.push(0);
            }
            while !buf.len().is_multiple_of(4) {
                buf.push(0);
            }
        }

        fn align4(len: usize) -> usize {
            (len + 3) & !3
        }

        fn last_errno() -> Error {
            Error::Io(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO))
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_set_channel_request() {
                let request = plan_set_channel(9, 5220).expect("valid request");

                assert_eq!(request.ifindex, 9);
                assert_eq!(request.frequency_mhz, 5220);
            }
        }
    }
}
