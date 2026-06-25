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

        let mut offset = 4;
        let mut present_words = Vec::new();
        loop {
            let word = read_field::<4>(frame, header_len, offset)?;
            let present = u32::from_le_bytes(word);
            present_words.push(present);
            offset += 4;
            if present & (1 << 31) == 0 {
                break;
            }
        }

        let mut tsft = None;
        let mut flags = None;
        let mut antenna_signal_dbm = None;

        'fields: for (word_index, present) in present_words.into_iter().enumerate() {
            for bit in 0..31 {
                if present & (1 << bit) == 0 {
                    continue;
                }

                let Some(field) = field_layout(word_index, bit) else {
                    break 'fields;
                };
                offset = align(offset, field.align);
                let bytes = read_slice(frame, header_len, offset, field.size)?;

                match (word_index, bit) {
                    (0, 0) => {
                        tsft = Some(u64::from_le_bytes(
                            bytes.try_into().expect("TSFT length is 8 bytes"),
                        ));
                    }
                    (0, 1) => flags = Some(bytes[0]),
                    (0, 5) => antenna_signal_dbm = Some(bytes[0] as i8),
                    _ => {}
                }

                offset += field.size;
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

    pub fn build_tx_header() -> Vec<u8> {
        vec![
            0x00, 0x00, 0x09, 0x00, // version, pad, length
            0x04, 0x00, 0x00, 0x00, // present: rate
            0x18, // 12 Mbps encoded as 500 kbps units
        ]
    }

    fn align(offset: usize, alignment: usize) -> usize {
        debug_assert!(alignment.is_power_of_two());
        (offset + alignment - 1) & !(alignment - 1)
    }

    #[derive(Clone, Copy)]
    struct FieldLayout {
        align: usize,
        size: usize,
    }

    fn field_layout(word_index: usize, bit: usize) -> Option<FieldLayout> {
        if word_index != 0 {
            return None;
        }

        let layout = match bit {
            0 => FieldLayout { align: 8, size: 8 },   // TSFT
            1 => FieldLayout { align: 1, size: 1 },   // flags
            2 => FieldLayout { align: 1, size: 1 },   // rate
            3 => FieldLayout { align: 2, size: 4 },   // channel
            4 => FieldLayout { align: 2, size: 2 },   // FHSS
            5 => FieldLayout { align: 1, size: 1 },   // dBm antenna signal
            6 => FieldLayout { align: 1, size: 1 },   // dBm antenna noise
            7 => FieldLayout { align: 2, size: 2 },   // lock quality
            8 => FieldLayout { align: 2, size: 2 },   // TX attenuation
            9 => FieldLayout { align: 2, size: 2 },   // dB TX attenuation
            10 => FieldLayout { align: 1, size: 1 },  // dBm TX power
            11 => FieldLayout { align: 1, size: 1 },  // antenna
            12 => FieldLayout { align: 1, size: 1 },  // dB antenna signal
            13 => FieldLayout { align: 1, size: 1 },  // dB antenna noise
            14 => FieldLayout { align: 2, size: 2 },  // RX flags
            15 => FieldLayout { align: 2, size: 2 },  // TX flags
            16 => FieldLayout { align: 1, size: 1 },  // RTS retries
            17 => FieldLayout { align: 1, size: 1 },  // data retries
            19 => FieldLayout { align: 1, size: 3 },  // MCS
            20 => FieldLayout { align: 4, size: 8 },  // A-MPDU status
            21 => FieldLayout { align: 2, size: 12 }, // VHT
            22 => FieldLayout { align: 8, size: 12 }, // timestamp
            29 | 30 => return None,                   // namespace switches are not data fields
            _ => return None,
        };
        Some(layout)
    }

    fn read_field<const N: usize>(
        frame: &[u8],
        header_len: usize,
        offset: usize,
    ) -> Result<[u8; N], RadiotapError> {
        let bytes = read_slice(frame, header_len, offset, N)?;
        Ok(bytes.try_into().expect("slice length matches field length"))
    }

    fn read_slice(
        frame: &[u8],
        header_len: usize,
        offset: usize,
        size: usize,
    ) -> Result<&[u8], RadiotapError> {
        let end = offset.checked_add(size).ok_or(RadiotapError::TooShort)?;
        frame
            .get(offset..end)
            .filter(|_| end <= header_len)
            .ok_or(RadiotapError::TooShort)
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

        #[test]
        fn parses_carl9170_header_with_extra_fields_and_no_tsft() {
            let frame = [
                0x00, 0x00, 0x12, 0x00, // version, pad, length
                0x2e, 0x48, 0x00,
                0x00, // present: flags, rate, channel, signal, antenna, rx flags
                0x10, // flags: frame includes FCS
                0x0c, // rate
                0x64, 0x14, 0x40, 0x01, // channel frequency 5220 + flags
                0xc3, // antenna signal: -61 dBm
                0x05, // antenna
                0x00, 0x00, // RX flags
                0xde, 0xad, // payload begins after radiotap header
            ];

            let parsed = parse_header(&frame).expect("valid carl9170 radiotap header");

            assert_eq!(parsed.header_len, 18);
            assert_eq!(parsed.payload, &[0xde, 0xad]);
            assert_eq!(parsed.tsft, None);
            assert_eq!(parsed.flags, Some(0x10));
            assert_eq!(parsed.antenna_signal_dbm, Some(-61));
        }

        #[test]
        fn builds_owl_compatible_tx_radiotap_header() {
            assert_eq!(
                build_tx_header(),
                vec![
                    0x00, 0x00, 0x09, 0x00, // version, pad, length
                    0x04, 0x00, 0x00, 0x00, // present: rate
                    0x18, // 12 Mbps encoded as 500 kbps units
                ]
            );
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
        let frame_control = u16::from_le_bytes([frame[0], frame[1]]);
        // QoS data subtypes carry a 2-byte QoS control field after the
        // sequence control; owl strips it before the frame body (owl/src/rx.c:536-541).
        let body_offset = if is_qos_data(frame_control) { 26 } else { 24 };
        if frame.len() < body_offset {
            return Err(ParseError::TooShort);
        }

        Ok(MacHeader {
            frame_control,
            duration_id: u16::from_le_bytes([frame[2], frame[3]]),
            destination: read_addr(frame, 4)?,
            source: read_addr(frame, 10)?,
            bssid: read_addr(frame, 16)?,
            sequence_control: u16::from_le_bytes([frame[22], frame[23]]),
            body: &frame[body_offset..],
        })
    }

    /// True for data frames whose subtype carries a QoS control field
    /// (802.11 STYPE_QOS_DATA bit, owl/src/rx.c:536).
    pub fn is_qos_data(frame_control: u16) -> bool {
        const FTYPE_DATA: u16 = 0x0008;
        const STYPE_QOS: u16 = 0x0080;
        (frame_control & (FTYPE_DATA | STYPE_QOS)) == (FTYPE_DATA | STYPE_QOS)
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

    pub fn build_data_frame(
        destination: [u8; 6],
        source: [u8; 6],
        bssid: [u8; 6],
        sequence_control: u16,
        body: &[u8],
    ) -> Vec<u8> {
        let mut frame = Vec::with_capacity(24 + body.len());
        frame.extend_from_slice(&0x0008u16.to_le_bytes());
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
        fn parses_qos_data_body_after_two_byte_qos_control() {
            // fc = 0x0888 (data + QoS data subtype). owl strips the 2-byte QoS
            // control so the LLC body starts at offset 26, not 24.
            let frame = [
                0x88, 0x08, // frame control (QoS data)
                0x00, 0x00, // duration
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // destination
                0x02, 0x11, 0x22, 0x33, 0x44, 0x55, // source
                0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, // bssid
                0x10, 0x00, // sequence control
                0x00, 0x00, // QoS control (stripped from body)
                0xaa, 0xaa, 0x03, // LLC/SNAP start
            ];

            let parsed = parse_mac_header(&frame).expect("valid QoS data header");

            assert_eq!(parsed.frame_control, 0x0888);
            assert!(is_qos_data(parsed.frame_control));
            assert_eq!(parsed.body, &[0xaa, 0xaa, 0x03]);
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

        #[test]
        fn builds_three_address_data_frame() {
            let frame = build_data_frame(
                [0x33, 0x33, 0, 0, 0, 1],
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                [0x00, 0x25, 0x00, 0xff, 0x94, 0x73],
                0x0020,
                &[0xaa, 0xaa],
            );

            assert_eq!(&frame[0..2], &[0x08, 0x00]);
            assert_eq!(&frame[4..10], &[0x33, 0x33, 0, 0, 0, 1]);
            assert_eq!(&frame[22..24], &[0x20, 0x00]);
            assert_eq!(&frame[24..], &[0xaa, 0xaa]);
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

    /// owl `awdl_handle_election_params_tlv` fields (owl/src/rx.c:111-131).
    #[derive(Debug, PartialEq, Eq)]
    pub struct ElectionParameters {
        pub height: u8,
        pub master_addr: [u8; 6],
        pub master_metric: u32,
        pub self_metric: u32,
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
        // owl iterates TLVs with read_tlv (owl/src/wire.c:209-222) inside a
        // `while ((len = read_tlv(..)) > 0)` loop (owl/src/rx.c:295): an
        // incomplete *trailing* TLV makes read_tlv return OUT_OF_BOUNDS and the
        // loop simply stops — the already-seen TLVs (and the peer, added
        // earlier) are kept. Mirror that tolerance here: stop cleanly on a
        // short trailing header or value rather than failing the whole frame.
        let mut offset = 0;
        let mut tlvs = Vec::new();
        while offset + 3 <= bytes.len() {
            let kind = bytes[offset];
            let len = u16::from_le_bytes([bytes[offset + 1], bytes[offset + 2]]) as usize;
            offset += 3;
            let Some(value) = bytes.get(offset..offset + len) else {
                break; // incomplete trailing TLV value
            };
            tlvs.push(Tlv { kind, value });
            offset += len;
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

    pub fn parse_election_parameters_v2(value: &[u8]) -> Result<ElectionParametersV2, ParseError> {
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

    /// owl `awdl_handle_election_params_tlv` reads (owl/src/rx.c:120-125):
    /// distance_to_master @3 (u8), master_addr @5, master_metric @11 (le32),
    /// self_metric @15 (le32).
    pub fn parse_election_parameters(value: &[u8]) -> Result<ElectionParameters, ParseError> {
        Ok(ElectionParameters {
            height: *value.get(3).ok_or(ParseError::TooShort)?,
            master_addr: read_addr(value, 5)?,
            master_metric: read_le32(value, 11)?,
            self_metric: read_le32(value, 15)?,
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

    pub fn encapsulate_ethernet_payload(ethernet: &[u8], seq: u16) -> Result<Vec<u8>, ParseError> {
        if ethernet.len() < 14 {
            return Err(ParseError::TooShort);
        }
        let ethertype = [ethernet[12], ethernet[13]];
        let payload = &ethernet[14..];
        let mut body = Vec::with_capacity(16 + payload.len());
        body.extend_from_slice(&[0xaa, 0xaa, 0x03, 0x00, 0x17, 0xf2, 0x08, 0x00]);
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&seq.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&ethertype);
        body.extend_from_slice(payload);
        Ok(body)
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
        fn parse_tlvs_stops_cleanly_on_incomplete_trailing_tlv() {
            // Two good TLVs followed by an incomplete trailing value, then a
            // short header fragment — owl's read_tlv loop exits on both.
            let tlvs = [
                0x04, 0x02, 0x00, 0xaa, 0xbb, // ok: sync params fragment
                0x18, 0x04, 0x00, 0x01, 0x02, 0x03, 0x04, // ok: election v2 fragment
                0x07, 0x08, 0x00, 0xce, 0x11, // incomplete: claims len 8 but only 2 bytes
                0xde, 0xad, // dangling header fragment (< 3 bytes)
            ];

            let parsed = parse_tlvs(&tlvs).expect("tolerant parse keeps leading TLVs");

            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[0].kind, 0x04);
            assert_eq!(parsed[1].kind, 0x18);
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
        fn parses_election_parameters_v1_tlv() {
            // layout per owl/src/rx.c:120-125: flags@0, id@1-2, distancetop@3,
            // unknown@4, master_addr@5, master_metric@11, self_metric@15, pad@19
            let mut value = [0u8; 21];
            value[3] = 4; // distance_to_master -> height
            value[5..11].copy_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            value[11..15].copy_from_slice(&900u32.to_le_bytes());
            value[15..19].copy_from_slice(&800u32.to_le_bytes());

            let parsed =
                parse_election_parameters(&value).expect("valid election parameters v1 TLV");

            assert_eq!(
                parsed,
                ElectionParameters {
                    height: 4,
                    master_addr: [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                    master_metric: 900,
                    self_metric: 800,
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
                    0x33, 0x33, 0x00, 0x00, 0x00, 0x01, 0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x86,
                    0xdd, 0x60, 0x00, 0x00, 0x00,
                ]
            );
        }

        #[test]
        fn builds_mif_vendor_action_body() {
            let body =
                build_action_body(ActionSubtype::Mif, 0x1122_3344, 0x5566_7788, &[0x04, 0, 0]);

            assert_eq!(
                body,
                vec![
                    0x7f, 0x00, 0x17, 0xf2, 0x08, 0x10, 0x03, 0x00, 0x44, 0x33, 0x22, 0x11, 0x88,
                    0x77, 0x66, 0x55, 0x04, 0x00, 0x00,
                ]
            );
        }

        #[test]
        fn encapsulates_ethernet_payload_as_awdl_data_body() {
            let ethernet = [
                0x33, 0x33, 0x00, 0x00, 0x00, 0x01, 0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x86, 0xdd,
                0x60, 0x00,
            ];

            let body = encapsulate_ethernet_payload(&ethernet, 7).expect("valid ethernet frame");

            assert_eq!(
                body,
                vec![
                    0xaa, 0xaa, 0x03, 0x00, 0x17, 0xf2, 0x08, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00,
                    0x00, 0x86, 0xdd, 0x60, 0x00,
                ]
            );
        }
    }
}

pub mod election {
    //! Faithful port of owl `election.c` / `election.h`.
    //!
    //! AWDL master election: each node advertises an election state and adopts
    //! the peer with the best `(master_counter, master_metric)`, breaking ties
    //! by shorter sync tree (height) then larger address. A node never syncs to
    //! a peer that already syncs to it (cycle prevention) and rejects trees
    //! taller than `TREE_MAX_HEIGHT`.

    /// owl `AWDL_ELECTION_METRIC_INIT` (owl/src/election.h:26).
    pub const METRIC_INIT: u32 = 60;
    /// owl `AWDL_ELECTION_COUNTER_INIT` (owl/src/election.h:27).
    pub const COUNTER_INIT: u32 = 0;
    /// owl `AWDL_ELECTION_TREE_MAX_HEIGHT` (owl/src/election.h:25).
    pub const TREE_MAX_HEIGHT: u32 = 10;

    /// owl `struct awdl_election_state` (owl/src/election.h:31-40).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ElectionState {
        pub master_addr: [u8; 6],
        pub sync_addr: [u8; 6],
        pub self_addr: [u8; 6],
        pub height: u32,
        pub master_metric: u32,
        pub self_metric: u32,
        pub master_counter: u32,
        pub self_counter: u32,
    }

    impl ElectionState {
        /// owl `awdl_election_state_init` (owl/src/election.c:52-58).
        pub fn new(self_addr: [u8; 6]) -> Self {
            Self {
                master_addr: self_addr,
                sync_addr: self_addr,
                self_addr,
                height: 0,
                master_metric: METRIC_INIT,
                self_metric: METRIC_INIT,
                master_counter: COUNTER_INIT,
                self_counter: COUNTER_INIT,
            }
        }

        /// owl `awdl_election_is_sync_master` (owl/src/election.c:35-37).
        pub fn is_sync_master(&self, addr: &[u8; 6]) -> bool {
            self.sync_addr == *addr
        }

        /// owl `awdl_election_run` (owl/src/election.c:67-120). Picks the best
        /// master from the supplied peer election views and updates self in
        /// place. `peers` need not be sorted; iteration order matches owl's.
        pub fn run(&mut self, peers: &[PeerElection]) {
            self.reset_self();

            // `master_state` starts as self (owl/src/election.c:71).
            let mut best_counter = self.self_counter;
            let mut best_metric = self.self_metric;
            let mut best_height = 0u32;
            let mut best_addr = self.self_addr;
            let mut adopted: Option<&PeerElection> = None;

            for peer in peers {
                if !peer.is_valid {
                    continue;
                }
                if peer.height + 1 > TREE_MAX_HEIGHT {
                    continue;
                }
                if peer.sync_addr == self.self_addr {
                    continue; // reject: would form a cycle in the sync tree
                }
                let cmp = (peer.master_counter, peer.master_metric)
                    .cmp(&(best_counter, best_metric));
                match cmp {
                    std::cmp::Ordering::Less => continue, // reject: lower master
                    std::cmp::Ordering::Equal => {
                        if peer.height > best_height {
                            continue; // reject: longer sync tree
                        } else if peer.height == best_height && peer.self_addr <= best_addr {
                            continue; // reject: smaller/equal address tie-break
                        }
                    }
                    std::cmp::Ordering::Greater => {}
                }
                // accept
                adopted = Some(peer);
                best_counter = peer.master_counter;
                best_metric = peer.master_metric;
                best_height = peer.height;
                best_addr = peer.self_addr;
            }

            if let Some(peer) = adopted {
                self.master_addr = peer.master_addr;
                self.sync_addr = peer.self_addr;
                self.master_metric = peer.master_metric;
                self.master_counter = peer.master_counter;
                self.height = peer.height + 1;
            }
        }

        /// owl `awdl_election_reset_self` (owl/src/election.c:44-50).
        fn reset_self(&mut self) {
            self.height = 0;
            self.master_addr = self.self_addr;
            self.sync_addr = self.self_addr;
            self.master_metric = self.self_metric;
            self.master_counter = self.self_counter;
        }
    }

    /// Read-only view of a peer's election state consumed by `ElectionState::run`.
    /// Mirrors the fields owl reads from `struct awdl_peer` during election.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PeerElection {
        pub self_addr: [u8; 6],
        pub sync_addr: [u8; 6],
        pub master_addr: [u8; 6],
        pub height: u32,
        pub master_metric: u32,
        pub master_counter: u32,
        pub is_valid: bool,
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn local(addr: [u8; 6]) -> ElectionState {
            ElectionState::new(addr)
        }

        /// A peer view that syncs to itself (is its own master) with the given
        /// advertised master counter/metric.
        fn peer_master(
            self_addr: [u8; 6],
            master_counter: u32,
            master_metric: u32,
            height: u32,
        ) -> PeerElection {
            PeerElection {
                self_addr,
                sync_addr: self_addr,
                master_addr: self_addr,
                height,
                master_metric,
                master_counter,
                is_valid: true,
            }
        }

        #[test]
        fn lone_node_is_self_master_with_init_values() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            state.run(&[]);
            assert_eq!(state.master_addr, [0x02, 0, 0, 0, 0, 1]);
            assert_eq!(state.sync_addr, [0x02, 0, 0, 0, 0, 1]);
            assert_eq!(state.master_metric, METRIC_INIT);
            assert_eq!(state.master_counter, COUNTER_INIT);
            assert_eq!(state.height, 0);
        }

        #[test]
        fn adopts_peer_with_higher_master_counter() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            let peer = peer_master([0x02, 0, 0, 0, 0, 2], 10, 200, 0);

            state.run(std::slice::from_ref(&peer));

            assert_eq!(state.master_addr, [0x02, 0, 0, 0, 0, 2]);
            assert_eq!(state.sync_addr, [0x02, 0, 0, 0, 0, 2]);
            assert_eq!(state.master_metric, 200);
            assert_eq!(state.master_counter, 10);
            assert_eq!(state.height, 1);
        }

        #[test]
        fn rejects_peer_with_lower_master_than_self() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            // self advertises (counter=0, metric=60); peer advertises metric 10 < 60
            let peer = peer_master([0x02, 0, 0, 0, 0, 2], 0, 10, 0);

            state.run(std::slice::from_ref(&peer));

            assert_eq!(state.sync_addr, state.self_addr); // still self-master
        }

        #[test]
        fn breaks_equal_master_tie_by_shorter_height_then_address() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            let tall = peer_master([0x02, 0, 0, 0, 0, 9], 5, 100, 3);
            let short_same = peer_master([0x02, 0, 0, 0, 0, 2], 5, 100, 0);
            let short_larger = peer_master([0x02, 0, 0, 0, 0, 3], 5, 100, 0);

            state.run(&[tall, short_same, short_larger]);

            // equal (counter,metric); short_larger wins on address tie-break
            assert_eq!(state.sync_addr, [0x02, 0, 0, 0, 0, 3]);
        }

        #[test]
        fn rejects_peer_that_syncs_to_self_no_cycles() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            // peer claims to sync to us -> would create a cycle
            let mut peer = peer_master([0x02, 0, 0, 0, 0, 2], 99, 9999, 0);
            peer.sync_addr = [0x02, 0, 0, 0, 0, 1];

            state.run(std::slice::from_ref(&peer));

            assert_eq!(state.sync_addr, state.self_addr); // rejected
        }

        #[test]
        fn rejects_peer_whose_tree_would_exceed_max_height() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            let peer = peer_master([0x02, 0, 0, 0, 0, 2], 99, 9999, TREE_MAX_HEIGHT);

            state.run(std::slice::from_ref(&peer));

            assert_eq!(state.sync_addr, state.self_addr); // rejected
        }

        #[test]
        fn ignores_invalid_peers() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            let mut peer = peer_master([0x02, 0, 0, 0, 0, 2], 99, 9999, 0);
            peer.is_valid = false;

            state.run(std::slice::from_ref(&peer));

            assert_eq!(state.sync_addr, state.self_addr); // ignored
        }

        #[test]
        fn is_sync_master_detects_elected_master() {
            let mut state = local([0x02, 0, 0, 0, 0, 1]);
            let peer_addr = [0x02, 0, 0, 0, 0, 2];
            state.run(std::slice::from_ref(&peer_master(peer_addr, 1, 200, 0)));

            assert!(state.is_sync_master(&peer_addr));
            assert!(!state.is_sync_master(&[0x02, 0, 0, 0, 0, 9]));
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

        pub fn sync_error_tu(&self, now_us: u64, time_to_next_aw_tu: u16, aw_counter: u16) -> i64 {
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

        /// owl `awdl_sync_next_aw_us` (owl/src/sync.c:40-45): microseconds to
        /// the next availability-window boundary.
        pub fn next_aw_us(&self, now_us: u64) -> u64 {
            let eaw_period_us = tu_to_us(
                u64::from(self.presence_mode) * u64::from(self.aw_period_tu),
            );
            let time_since = now_us.saturating_sub(self.last_update_us);
            eaw_period_us - (time_since % eaw_period_us)
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

pub mod schedule {
    //! Faithful port of owl `schedule.c` / `schedule.h`: availability-window
    //! gating for action/data transmission.
    use crate::sync::SyncState;

    pub const CHANSEQ_LEN: usize = 16;
    /// owl `AWDL_MULTICAST_GUARD_TU` (owl/src/schedule.h:29).
    pub const MULTICAST_GUARD_TU: u16 = 16;

    const TU_US: u64 = 1024;

    /// owl `awdl_is_multicast_eaw` (owl/src/schedule.c:43-46): slots 0 and 10
    /// are the AWDL multicast availability windows.
    pub fn is_multicast_eaw(sync: &SyncState, now_us: u64) -> bool {
        let slot = usize::from(sync.current_eaw(now_us) % CHANSEQ_LEN as u16);
        slot == 0 || slot == 10
    }

    /// Channel number (opclass encoding) for a sequence slot, or 0 if null.
    pub fn slot_channel(channel_sequence: &[[u8; 2]; CHANSEQ_LEN], slot: usize) -> u8 {
        channel_sequence[slot % CHANSEQ_LEN][0]
    }

    /// owl `awdl_can_send_in` (owl/src/schedule.c:48-55): returns 0 when it is
    /// safe to send now (inside the guard-free core of the AW), a positive
    /// number of seconds to wait before the next safe window, or a negative
    /// number of seconds meaning "we are in the trailing guard, wait this long
    /// plus the guard before retrying".
    pub fn can_send_in(sync: &SyncState, now_us: u64, guard_tu: u16) -> i64 {
        let next_aw = sync.next_aw_us(now_us);
        let guard = tu_to_us(u64::from(guard_tu));
        let eaw = tu_to_us(64); // presence_mode(4) * aw_period(16) TU
        if next_aw < guard {
            -(us_to_sec(guard - next_aw))
        } else if eaw - next_aw < guard {
            us_to_sec(guard - (eaw - next_aw))
        } else {
            0
        }
    }

    fn tu_to_us(tu: u64) -> u64 {
        tu * TU_US
    }

    fn us_to_sec(us: u64) -> i64 {
        us as i64 / 1_000_000
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn synced() -> SyncState {
            SyncState::new(1_000_000)
        }

        #[test]
        fn mid_eaw_is_sendable_outside_guard() {
            // last_update at 1_000_000; at 1_032_000 we are ~32 TU into the EAW
            // (well clear of the 16 TU guard on both sides).
            let sync = synced();
            assert_eq!(can_send_in(&sync, 1_032_768, MULTICAST_GUARD_TU), 0);
        }

        #[test]
        fn detects_multicast_eaw_slots() {
            let sync = synced();
            // current_eaw advances with time; verify slot 0 at the start.
            assert!(is_multicast_eaw(&sync, 1_000_000));
        }

        #[test]
        fn slot_channel_reads_chan_num_byte() {
            let seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert_eq!(slot_channel(&seq, 0), 44);
            assert_eq!(slot_channel(&seq, 15), 44);
        }
    }
}

pub mod peers {
    //! Faithful port of owl `peers.c` / `peers.h`.
    use crate::election::{ElectionState, PeerElection};

    /// owl `AWDL_CHANSEQ_LENGTH` (owl/src/channel.h:25).
    pub const CHANSEQ_LEN: usize = 16;
    /// owl `HOST_NAME_LENGTH_MAX` (owl/src/peers.h:29).
    pub const HOST_NAME_MAX: usize = 64;
    /// owl `PEERS_DEFAULT_TIMEOUT` (owl/src/peers.c:28). The peer table stores
    /// `last_update` in microseconds (set from the RX timestamp), so the value
    /// owl labels "ms" is applied as microseconds: a peer unheard for ~2 s is
    /// dropped.
    pub const PEER_TIMEOUT_US: u64 = 2_000_000;

    /// owl `struct awdl_peer` (owl/src/peers.h:38-52).
    #[derive(Debug, Clone)]
    pub struct Peer {
        pub addr: [u8; 6],
        pub last_update_us: u64,
        pub election: ElectionState,
        /// Channel sequence, each slot a `(chan_num, opclass)` pair (opclass
        /// encoding). `owl awdl_chan` is a 2-byte union.
        pub sequence: [[u8; 2]; CHANSEQ_LEN],
        pub sync_offset_us: u64,
        pub name: String,
        pub country_code: [u8; 3],
        pub infra_addr: [u8; 6],
        pub version: u8,
        pub devclass: u8,
        pub supports_v2: bool,
        pub sent_mif: bool,
        pub is_valid: bool,
    }

    impl Peer {
        /// owl `awdl_peer_new` (owl/src/peers.c:66-81).
        pub fn new(addr: [u8; 6]) -> Self {
            Self {
                addr,
                last_update_us: 0,
                election: ElectionState::new(addr),
                sequence: [[0, 0]; CHANSEQ_LEN],
                sync_offset_us: 0,
                name: String::new(),
                country_code: [b'N', b'A', 0],
                infra_addr: [0; 6],
                version: 0,
                devclass: 0,
                supports_v2: false,
                sent_mif: false,
                is_valid: false,
            }
        }

        /// owl `awdl_peer_is_valid` (owl/src/peers.c:62-64): a peer becomes a
        /// candidate for election/data once it has sent a MIF, a version, and a
        /// device class.
        pub fn is_complete(&self) -> bool {
            self.sent_mif && self.devclass != 0 && self.version != 0
        }

        /// Read-only election view consumed by `ElectionState::run`.
        pub fn election_view(&self) -> PeerElection {
            PeerElection {
                self_addr: self.addr,
                sync_addr: self.election.sync_addr,
                master_addr: self.election.master_addr,
                height: self.election.height,
                master_metric: self.election.master_metric,
                master_counter: self.election.master_counter,
                is_valid: self.is_valid,
            }
        }
    }

    /// owl `struct awdl_peer_state` and its hashmap (owl/src/peers.c).
    #[derive(Debug, Default)]
    pub struct PeerTable {
        peers: Vec<Peer>,
    }

    impl PeerTable {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn len(&self) -> usize {
            self.peers.len()
        }

        pub fn is_empty(&self) -> bool {
            self.peers.is_empty()
        }

        pub fn get(&self, addr: &[u8; 6]) -> Option<&Peer> {
            self.peers.iter().find(|p| &p.addr == addr)
        }

        pub fn get_mut(&mut self, addr: &[u8; 6]) -> Option<&mut Peer> {
            self.peers.iter_mut().find(|p| &p.addr == addr)
        }

        /// owl `awdl_peer_add` (owl/src/peers.c:83-116): create or update the
        /// peer, refresh `last_update`, and recompute validity. Returns `true`
        /// iff the peer transitioned to valid on this call (owl fires its
        /// `peer_cb` then).
        pub fn touch(&mut self, addr: [u8; 6], now_us: u64) -> bool {
            if let Some(idx) = self.peers.iter().position(|p| p.addr == addr) {
                let peer = &mut self.peers[idx];
                let was_valid = peer.is_valid;
                peer.last_update_us = now_us;
                peer.is_valid = peer.is_complete();
                peer.is_valid && !was_valid
            } else {
                let mut peer = Peer::new(addr);
                peer.last_update_us = now_us;
                peer.is_valid = peer.is_complete();
                let became = peer.is_valid;
                self.peers.push(peer);
                became
            }
        }

        pub fn iter(&self) -> std::slice::Iter<'_, Peer> {
            self.peers.iter()
        }

        /// Election inputs for all peers (owl feeds the whole table to
        /// `awdl_election_run`).
        pub fn election_views(&self) -> Vec<PeerElection> {
            self.peers.iter().map(Peer::election_view).collect()
        }

        /// owl `awdl_peers_remove` (owl/src/peers.c:170-187): drop peers whose
        /// `last_update` is strictly older than `cutoff_us`. Returns the removed
        /// addresses (owl invokes `peer_remove_cb` for each).
        pub fn remove_stale(&mut self, cutoff_us: u64) -> Vec<[u8; 6]> {
            let mut removed = Vec::new();
            self.peers.retain(|p| {
                if p.last_update_us < cutoff_us {
                    removed.push(p.addr);
                    false
                } else {
                    true
                }
            });
            removed
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn touch_creates_then_updates_peer() {
            let mut table = PeerTable::new();
            let addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];

            assert!(!table.touch(addr, 1_000));
            assert_eq!(table.len(), 1);
            assert_eq!(table.get(&addr).unwrap().last_update_us, 1_000);

            assert!(!table.touch(addr, 2_000));
            assert_eq!(table.len(), 1);
            assert_eq!(table.get(&addr).unwrap().last_update_us, 2_000);
        }

        #[test]
        fn peer_becomes_valid_only_with_mif_version_and_devclass() {
            let mut table = PeerTable::new();
            let addr = [0x02, 0, 0, 0, 0, 1];
            table.touch(addr, 0);

            table.get_mut(&addr).unwrap().sent_mif = true;
            assert!(!table.touch(addr, 10)); // still missing version/devclass

            table.get_mut(&addr).unwrap().version = 0x34;
            assert!(!table.touch(addr, 20)); // still missing devclass

            table.get_mut(&addr).unwrap().devclass = 1;
            assert!(table.touch(addr, 30)); // now complete -> valid transition
            assert!(table.get(&addr).unwrap().is_valid);
        }

        #[test]
        fn remove_stale_drops_old_peers_keeps_recent() {
            let mut table = PeerTable::new();
            let old = [0x02, 0, 0, 0, 0, 1];
            let recent = [0x02, 0, 0, 0, 0, 2];
            table.touch(old, 1_000);
            table.touch(recent, 5_000);

            let removed = table.remove_stale(4_000);

            assert_eq!(removed, vec![old]);
            assert!(table.get(&old).is_none());
            assert!(table.get(&recent).is_some());
        }

        #[test]
        fn election_views_carry_validity_and_addrs() {
            let mut table = PeerTable::new();
            let addr = [0x02, 0, 0, 0, 0, 7];
            table.touch(addr, 100);
            {
                let p = table.get_mut(&addr).unwrap();
                p.sent_mif = true;
                p.version = 0x34;
                p.devclass = 1;
                p.election.master_counter = 42;
            }
            table.touch(addr, 200); // recompute validity

            let views = table.election_views();
            assert_eq!(views.len(), 1);
            assert!(views[0].is_valid);
            assert_eq!(views[0].self_addr, addr);
            assert_eq!(views[0].master_counter, 42);
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
        pub pcap_path: Option<String>,
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
                pcap_path: None,
            };

            assert_eq!(validate_config(config.clone()), Ok(config));
        }
    }
}

pub mod pcap {
    use std::io::{self, Write};

    pub const DLT_IEEE802_11_RADIOTAP: u32 = 127;

    pub struct Writer<W: Write> {
        inner: W,
    }

    impl<W: Write> Writer<W> {
        pub fn new(inner: W) -> io::Result<Self> {
            let mut writer = Self { inner };
            writer.write_global_header()?;
            Ok(writer)
        }

        pub fn write_packet(&mut self, ts_us: u64, frame: &[u8]) -> io::Result<()> {
            let ts_sec = (ts_us / 1_000_000) as u32;
            let ts_usec = (ts_us % 1_000_000) as u32;
            let len = frame.len() as u32;
            self.inner.write_all(&ts_sec.to_le_bytes())?;
            self.inner.write_all(&ts_usec.to_le_bytes())?;
            self.inner.write_all(&len.to_le_bytes())?;
            self.inner.write_all(&len.to_le_bytes())?;
            self.inner.write_all(frame)
        }

        fn write_global_header(&mut self) -> io::Result<()> {
            self.inner.write_all(&0xa1b2c3d4u32.to_le_bytes())?;
            self.inner.write_all(&2u16.to_le_bytes())?;
            self.inner.write_all(&4u16.to_le_bytes())?;
            self.inner.write_all(&0i32.to_le_bytes())?;
            self.inner.write_all(&0u32.to_le_bytes())?;
            self.inner.write_all(&65_535u32.to_le_bytes())?;
            self.inner.write_all(&DLT_IEEE802_11_RADIOTAP.to_le_bytes())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn writes_little_endian_radiotap_pcap() {
            let mut bytes = Vec::new();
            {
                let mut writer = Writer::new(&mut bytes).expect("global header written");
                writer
                    .write_packet(1_234_567, &[0x00, 0x00, 0x08, 0x00, 0xd0])
                    .expect("packet written");
            }

            assert_eq!(&bytes[0..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
            assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 2);
            assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 4);
            assert_eq!(
                u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
                65_535
            );
            assert_eq!(
                u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
                DLT_IEEE802_11_RADIOTAP
            );
            assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 1);
            assert_eq!(
                u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
                234_567
            );
            assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 5);
            assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 5);
            assert_eq!(&bytes[40..45], &[0x00, 0x00, 0x08, 0x00, 0xd0]);
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
        Radiotap(radiotap::RadiotapError),
        BadFcs,
        FcsTooShort,
        Ieee80211(ieee80211::ParseError),
        AwdlAction(awdl::ParseError),
        AwdlTlv(awdl::ParseError),
    }

    const RADIOTAP_F_FCS: u8 = 0x10;
    const RADIOTAP_F_BADFCS: u8 = 0x40;

    pub fn parse_action_frame<'a>(
        frame: &'a [u8],
        host_now_us: u64,
        bridge: &mut TsftBridge,
    ) -> Result<ParsedActionFrame<'a>, ParseError> {
        let radiotap = radiotap::parse_header(frame).map_err(ParseError::Radiotap)?;
        let ieee80211_payload = ieee80211_payload_without_fcs(&radiotap)?;
        let mac = ieee80211::parse_mac_header(ieee80211_payload).map_err(ParseError::Ieee80211)?;
        let action = awdl::parse_action_frame(mac.body).map_err(ParseError::AwdlAction)?;
        let tlvs = awdl::parse_tlvs(action.tlvs).map_err(ParseError::AwdlTlv)?;
        let rx_time_us = radiotap
            .tsft
            .map(|hw_tsft| bridge.timestamp_rx(hw_tsft, host_now_us))
            .unwrap_or(host_now_us);

        Ok(ParsedActionFrame {
            rx_time_us,
            rssi_dbm: radiotap.antenna_signal_dbm,
            source: mac.source,
            destination: mac.destination,
            action,
            tlvs,
        })
    }

    pub fn ieee80211_payload_without_fcs<'a>(
        radiotap: &radiotap::RadiotapHeader<'a>,
    ) -> Result<&'a [u8], ParseError> {
        let flags = radiotap.flags.unwrap_or(0);
        if flags & RADIOTAP_F_BADFCS != 0 {
            return Err(ParseError::BadFcs);
        }
        if flags & RADIOTAP_F_FCS != 0 {
            return radiotap
                .payload
                .get(
                    ..radiotap
                        .payload
                        .len()
                        .checked_sub(4)
                        .ok_or(ParseError::FcsTooShort)?,
                )
                .ok_or(ParseError::FcsTooShort);
        }
        Ok(radiotap.payload)
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

        #[test]
        fn falls_back_to_host_time_when_radiotap_has_no_tsft() {
            let mut frame = vec![
                0x00, 0x00, 0x12, 0x00, // radiotap version, pad, len
                0x2e, 0x48, 0x00, 0x00, // flags, rate, channel, signal, antenna, rx flags
                0x00, // flags
                0x0c, // rate
                0x64, 0x14, 0x40, 0x01, // channel frequency 5220 + flags
                0xc3, // -61 dBm
                0x05, // antenna
                0x00, 0x00, // RX flags
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
            let parsed = parse_action_frame(&frame, 202_000, &mut bridge)
                .expect("valid RX AWDL action frame without TSFT");

            assert_eq!(parsed.rx_time_us, 202_000);
            assert_eq!(parsed.rssi_dbm, Some(-61));
            assert_eq!(parsed.action.subtype, awdl::ActionSubtype::Mif);
        }

        #[test]
        fn strips_radiotap_fcs_before_parsing_action_tlvs() {
            let mut frame = vec![
                0x00, 0x00, 0x0a, 0x00, // radiotap version, pad, len
                0x02, 0x00, 0x00, 0x00, // present: flags
                0x10, // flags: frame includes trailing FCS
                0x00, // pad to radiotap len
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
                0x04, 0x02, 0x00, 0xaa, 0xbb, // one TLV
                0xde, 0xad, 0xbe, 0xef, // trailing FCS
            ]);

            let mut bridge = TsftBridge::default();
            let parsed = parse_action_frame(&frame, 303_000, &mut bridge)
                .expect("valid RX AWDL action frame with trailing FCS");

            assert_eq!(parsed.tlvs.len(), 1);
            assert_eq!(parsed.tlvs[0].kind, 0x04);
            assert_eq!(parsed.tlvs[0].value, &[0xaa, 0xbb]);
        }

        #[test]
        fn rejects_radiotap_bad_fcs_before_action_parse() {
            let mut frame = vec![
                0x00, 0x00, 0x0a, 0x00, // radiotap version, pad, len
                0x02, 0x00, 0x00, 0x00, // present: flags
                0x40, // flags: bad FCS
                0x00, // pad to radiotap len
            ];
            frame.extend_from_slice(&[
                0xd0, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0x00, 0x25, 0x00, 0xff, 0x94, 0x73, 0x00, 0x00,
            ]);

            let mut bridge = TsftBridge::default();
            let err =
                parse_action_frame(&frame, 303_000, &mut bridge).expect_err("bad FCS rejected");

            assert_eq!(err, ParseError::BadFcs);
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
        #[arg(long = "help", action = clap::ArgAction::Help, help = "Print help")]
        _help: Option<bool>,
        #[arg(short = 'i')]
        monitor_iface: String,
        #[arg(short = 'c', default_value_t = 44)]
        anchor_channel: u16,
        #[arg(short = 'h', default_value = "awdl0")]
        host_iface: String,
        #[arg(short = 'N')]
        assume_monitor: bool,
        #[arg(long = "pcap")]
        pcap_path: Option<String>,
    }

    pub fn parse_config_from_env() -> Result<iface::Config, CliError> {
        let args = Args::parse();
        config_from_args(args)
    }

    pub fn parse_config_from<I, S>(args: I) -> Result<iface::Config, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString> + Clone,
    {
        let args = Args::try_parse_from(args).map_err(|_| CliError::Invalid)?;
        config_from_args(args)
    }

    fn config_from_args(args: Args) -> Result<iface::Config, CliError> {
        iface::validate_config(iface::Config {
            monitor_iface: args.monitor_iface,
            host_iface: args.host_iface,
            anchor_channel: args.anchor_channel,
            assume_monitor: args.assume_monitor,
            pcap_path: args.pcap_path.or_else(|| std::env::var("FILIN_PCAP").ok()),
        })
        .map_err(|_| CliError::Invalid)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_owl_compatible_interface_flags() {
            let config =
                parse_config_from(["filin", "-i", "wlan0mon", "-c", "44", "-h", "awdl0", "-N"])
                    .expect("valid CLI args");

            assert_eq!(
                config,
                iface::Config {
                    monitor_iface: "wlan0mon".into(),
                    host_iface: "awdl0".into(),
                    anchor_channel: 44,
                    assume_monitor: true,
                    pcap_path: None,
                }
            );
        }

        #[test]
        fn parses_ignored_frame_pcap_path() {
            let config = parse_config_from([
                "filin",
                "-i",
                "wlan0mon",
                "-h",
                "awdl0",
                "--pcap",
                "/tmp/filin-ignored.pcap",
            ])
            .expect("valid CLI args");

            assert_eq!(config.pcap_path, Some("/tmp/filin-ignored.pcap".into()));
        }

        #[test]
        fn supports_long_help_without_reclaiming_short_h() {
            let err = Args::try_parse_from(["filin", "--help"]).expect_err("help exits parser");

            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
            assert!(err.to_string().contains("Usage:"));

            let config = parse_config_from(["filin", "-i", "wlan0mon", "-h", "awdl-test"])
                .expect("short -h remains host iface");
            assert_eq!(config.host_iface, "awdl-test");
        }
    }
}

pub mod state {
    //! Faithful port of owl `state.c` / `state.h`: aggregates the node's
    //! election, sync, channel, and peer-table state and applies received
    //! action-frame TLVs the way owl's `awdl_rx_action` does.
    use crate::{
        awdl,
        election::ElectionState,
        peers::{self, Peer, PeerTable, CHANSEQ_LEN},
        sync::SyncState,
    };

    /// owl `awdl_version(3, 4)` (owl/src/state.c:34).
    pub const AWDL_VERSION: u8 = 0x34;
    /// owl `AWDL_DEVCLASS_MACOS` (owl/src/version.h:34).
    pub const AWDL_DEVCLASS_MACOS: u8 = 1;

    /// owl `struct awdl_state` (owl/src/state.h:50-85), minus I/O callbacks.
    pub struct AwdlState {
        pub self_addr: [u8; 6],
        pub name: String,
        pub version: u8,
        pub dev_class: u8,
        pub election: ElectionState,
        pub sync: SyncState,
        pub peers: PeerTable,
        pub anchor_channel: u16,
        /// Own channel sequence as `(chan_num, opclass)` pairs. owl fills this
        /// with the master channel via `awdl_chanseq_init_static`. When filin
        /// syncs to a master, it adopts the master's sequence so they hop in
        /// lockstep.
        pub channel_sequence: [[u8; 2]; CHANSEQ_LEN],
        /// owl `state->channel.current` — the channel the monitor is currently
        /// tuned to (0 = not yet set / null slot).
        pub current_channel: u8,
        pub tx_seq: u16,
    }

    impl AwdlState {
        /// owl `awdl_init_state` (owl/src/state.c:30-67).
        pub fn new(
            self_addr: [u8; 6],
            name: String,
            anchor_channel: u16,
            now_us: u64,
        ) -> Self {
            let opclass = chan_opclass(anchor_channel);
            Self {
                self_addr,
                name,
                version: AWDL_VERSION,
                dev_class: AWDL_DEVCLASS_MACOS,
                election: ElectionState::new(self_addr),
                sync: SyncState::new(now_us),
                peers: PeerTable::new(),
                anchor_channel,
                channel_sequence: [[anchor_channel as u8, opclass]; CHANSEQ_LEN],
                current_channel: 0,
                tx_seq: 0,
            }
        }

        /// Apply a received action frame: register/update the originating peer,
        /// fold every TLV into that peer's state (election, chanseq, version,
        /// …), mark MIF, gate sync to the elected master, then re-run election.
        /// Mirrors owl `awdl_rx_action` (owl/src/rx.c:253-320) and
        /// `awdl_handle_*_tlv`.
        pub fn apply_action(
            &mut self,
            src: [u8; 6],
            subtype: awdl::ActionSubtype,
            tlvs: &[awdl::Tlv<'_>],
            rx_time_us: u64,
        ) {
            self.peers.touch(src, rx_time_us);
            let mut sync_update: Option<(u16, u16)> = None;
            if let Some(peer) = self.peers.get_mut(&src) {
                for tlv in tlvs {
                    match tlv.kind {
                        4 => {
                            if let Ok(p) = awdl::parse_sync_parameters(tlv.value) {
                                sync_update = Some((p.time_to_next_aw_tu, p.aw_counter));
                            }
                        }
                        5 if !peer.supports_v2 => apply_election_v1(peer, tlv.value),
                        18 => apply_chanseq(peer, tlv.value),
                        24 => apply_election_v2(peer, tlv.value),
                        16 => apply_arpa(peer, tlv.value),
                        21 => apply_version(peer, tlv.value),
                        _ => {}
                    }
                }
                if subtype == awdl::ActionSubtype::Mif {
                    peer.sent_mif = true;
                }
            }
            // owl only adopts sync from the elected sync master (rx.c:39).
            if let Some((time_to_next_aw_tu, aw_counter)) = sync_update {
                if self.election.is_sync_master(&src) {
                    let error_tu =
                        self.sync
                            .sync_error_tu(rx_time_us, time_to_next_aw_tu, aw_counter);
                    self.sync
                        .update_from_master(rx_time_us, time_to_next_aw_tu, aw_counter);
                    tracing::trace!(
                        error_tu,
                        aw_counter,
                        time_to_next_aw_tu,
                        "sync_error"
                    );
                }
            }
            // recompute validity (owl awdl_peer_add after TLVs, rx.c:317) and
            // run election (owl awdl_clean_peers, core.c:333).
            self.peers.touch(src, rx_time_us);
            self.election.run(&self.peers.election_views());
            // Adopt the elected master's channel sequence so filin hops to the
            // same channels as the peer. When filin is self-master, keep the
            // default static anchor sequence.
            self.sync_channel_sequence();
        }

        /// If we have an elected master with a non-trivial channel sequence,
        /// adopt it as our own. When we lose the master, revert to the static
        /// anchor sequence. This ensures filin is on the same channel as the
        /// peer during each EAW slot.
        fn sync_channel_sequence(&mut self) {
            let master = self.election.sync_addr;
            if master == self.self_addr {
                // self-master: use static anchor sequence
                let opclass = chan_opclass(self.anchor_channel);
                let anchor_seq = [[self.anchor_channel as u8, opclass]; CHANSEQ_LEN];
                if self.channel_sequence != anchor_seq {
                    self.channel_sequence = anchor_seq;
                    tracing::debug!("reverted to static anchor channel sequence");
                }
            } else if let Some(peer) = self.peers.get(&master) {
                // adopted master: copy its sequence if it has any non-null slots
                if peer.sequence.iter().any(|s| s[0] != 0)
                    && self.channel_sequence != peer.sequence
                {
                    self.channel_sequence = peer.sequence;
                    tracing::info!(
                        master = ?master,
                        "adopted master channel sequence"
                    );
                }
            }
        }

        /// owl `awdl_chan_num(state->channel.sequence[slot], enc)` — the channel
        /// number for the current EAW slot from our own sequence.
        pub fn desired_channel(&self, now_us: u64) -> u8 {
            let slot = usize::from(self.sync.current_eaw(now_us) % CHANSEQ_LEN as u16);
            self.channel_sequence[slot][0]
        }

        /// owl `awdl_switch_channel` decision (owl/daemon/core.c:279-308):
        /// returns `Some(channel)` if the monitor must be retuned, `None` if
        /// no switch is needed (same channel, or null slot).
        pub fn update_channel(&mut self, now_us: u64) -> Option<u16> {
            let desired = self.desired_channel(now_us);
            if desired == self.current_channel {
                return None;
            }
            self.current_channel = desired;
            if desired > 0 {
                Some(u16::from(desired))
            } else {
                None
            }
        }

        /// owl `awdl_clean_peers` (owl/daemon/core.c:320-336): drop stale peers
        /// then re-run election. Returns the removed peer addresses.
        pub fn clean_peers(&mut self, now_us: u64) -> Vec<[u8; 6]> {
            let cutoff = now_us.saturating_sub(peers::PEER_TIMEOUT_US);
            let removed = self.peers.remove_stale(cutoff);
            self.election.run(&self.peers.election_views());
            self.sync_channel_sequence();
            removed
        }

        /// owl `awdl_state_next_sequence_number` (owl/src/state.c:79-81).
        pub fn next_data_seq(&mut self) -> u16 {
            let s = self.tx_seq;
            self.tx_seq = self.tx_seq.wrapping_add(1);
            s
        }
    }

    /// owl `awdl_handle_election_params_tlv` (owl/src/rx.c:111-131).
    fn apply_election_v1(peer: &mut Peer, value: &[u8]) {
        if let Ok(e) = awdl::parse_election_parameters(value) {
            peer.election.height = u32::from(e.height);
            peer.election.master_addr = e.master_addr;
            peer.election.master_metric = e.master_metric;
            peer.election.self_metric = e.self_metric;
        }
    }

    /// owl `awdl_handle_election_params_v2_tlv` (owl/src/rx.c:133-149).
    fn apply_election_v2(peer: &mut Peer, value: &[u8]) {
        if let Ok(e) = awdl::parse_election_parameters_v2(value) {
            peer.election.master_addr = e.master_addr;
            peer.election.sync_addr = e.sync_addr;
            peer.election.master_counter = e.master_counter;
            peer.election.height = e.distance_to_master;
            peer.election.master_metric = e.master_metric;
            peer.election.self_metric = e.self_metric;
            peer.election.self_counter = e.self_counter;
            peer.supports_v2 = true;
        }
    }

    /// owl `awdl_handle_chanseq_tlv` (owl/src/rx.c:60-109): store the peer's
    /// 16-slot channel sequence (as raw `(chan_num, opclass)` bytes).
    fn apply_chanseq(peer: &mut Peer, value: &[u8]) {
        if let Ok(seq) = awdl::parse_channel_sequence(value) {
            for (slot, ch) in seq.channels.iter().take(CHANSEQ_LEN).enumerate() {
                peer.sequence[slot] = ch.to_le_bytes();
            }
        }
    }

    /// owl `awdl_handle_arpa_tlv` (owl/src/rx.c:151-158).
    fn apply_arpa(peer: &mut Peer, value: &[u8]) {
        if value.len() >= 2 {
            let name_len = usize::from(value[1]);
            let end = 2 + name_len;
            if end <= value.len() {
                peer.name = String::from_utf8_lossy(&value[2..end]).into_owned();
            }
        }
    }

    /// owl `awdl_handle_version_tlv` (owl/src/rx.c:196-206).
    fn apply_version(peer: &mut Peer, value: &[u8]) {
        if let (Some(v), Some(d)) = (value.first(), value.get(1)) {
            peer.version = *v;
            peer.devclass = *d;
        }
    }

    /// opclass byte for an AWDL social channel (owl `CHAN_OPCLASS_*`,
    /// owl/src/channel.h:51-53).
    pub fn chan_opclass(channel: u16) -> u8 {
        match channel {
            6 => 0x51,
            44 | 149 => 0x80,
            _ => 0x80,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn mif() -> awdl::ActionSubtype {
            awdl::ActionSubtype::Mif
        }

        fn version_tlv(version: u8, devclass: u8) -> awdl::Tlv<'static> {
            awdl::Tlv {
                kind: 21,
                value: Box::leak(vec![version, devclass].into_boxed_slice()),
            }
        }

        fn election_v2_tlv(master_counter: u32, metric: u32) -> awdl::Tlv<'static> {
            let mut value = vec![0u8; 40];
            value[0..6].copy_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]); // master
            value[6..12].copy_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]); // sync (self)
            value[12..16].copy_from_slice(&master_counter.to_le_bytes());
            value[16..20].copy_from_slice(&0u32.to_le_bytes()); // distance
            value[20..24].copy_from_slice(&metric.to_le_bytes());
            value[24..28].copy_from_slice(&metric.to_le_bytes());
            value[36..40].copy_from_slice(&0u32.to_le_bytes());
            awdl::Tlv { kind: 24, value: value.leak() }
        }

        #[test]
        fn state_inits_self_master_and_static_chanseq() {
            let state = AwdlState::new([0x02, 1, 2, 3, 4, 5], "filin".into(), 44, 0);
            assert_eq!(state.election.master_addr, [0x02, 1, 2, 3, 4, 5]);
            assert_eq!(state.election.master_metric, ElectionState::new([0; 6]).master_metric);
            assert_eq!(state.channel_sequence[0], [44, 0x80]);
            assert_eq!(state.version, AWDL_VERSION);
            assert_eq!(state.dev_class, AWDL_DEVCLASS_MACOS);
        }

        #[test]
        fn apply_action_registers_peer_and_adopts_master() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 1_000_000);
            let peer_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
            // peer advertises a high master counter; needs version+devclass+mif
            // to become valid before election will adopt it.
            let tlvs = vec![
                election_v2_tlv(50, 200),
                version_tlv(0x34, 1),
            ];
            state.apply_action(peer_addr, mif(), &tlvs, 1_100_000);

            let peer = state.peers.get(&peer_addr).unwrap();
            assert!(peer.is_valid);
            assert!(peer.sent_mif);
            assert_eq!(peer.version, 0x34);
            // election should have adopted the peer as master
            assert_eq!(state.election.sync_addr, peer_addr);
            assert_eq!(state.election.master_counter, 50);
            assert_eq!(state.election.height, 1);
        }

        #[test]
        fn sync_only_adopted_from_elected_master() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 1_000_000);
            let master_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
            let other_addr = [0x02, 0, 0, 0, 0, 9];

            // make master_addr the elected master first
            state.apply_action(
                master_addr,
                mif(),
                &[election_v2_tlv(50, 200), version_tlv(0x34, 1)],
                1_100_000,
            );
            assert!(state.election.is_sync_master(&master_addr));
            let aw_counter_before = state.sync.aw_counter;

            // a sync TLV from a non-master must NOT move our clock
            let mut sync_val = vec![0u8; 31];
            sync_val[1..3].copy_from_slice(&10u16.to_le_bytes());
            sync_val[29..31].copy_from_slice(&16u16.to_le_bytes());
            let non_master_tlv = awdl::Tlv { kind: 4, value: sync_val.leak() };
            state.apply_action(other_addr, mif(), std::slice::from_ref(&non_master_tlv), 1_200_000);
            assert_eq!(state.sync.aw_counter, aw_counter_before, "non-master must not move sync");
        }

        #[test]
        fn clean_peers_evicts_stale_and_reruns_election() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
            state.apply_action(
                peer_addr,
                mif(),
                &[election_v2_tlv(50, 200), version_tlv(0x34, 1)],
                1_000,
            );
            assert_eq!(state.election.sync_addr, peer_addr);

            // after the timeout with no further updates the peer is gone and we
            // fall back to being our own master
            state.clean_peers(1_000 + peers::PEER_TIMEOUT_US + 1);
            assert!(state.peers.get(&peer_addr).is_none());
            assert_eq!(state.election.sync_addr, state.election.self_addr);
        }

        #[test]
        fn adopts_master_channel_sequence_on_election() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];

            // build a chanseq TLV value with mixed channels
            let mut chanseq_val = vec![15, 3, 0, 3];
            chanseq_val.extend_from_slice(&0xffffu16.to_le_bytes());
            for i in 0..16 {
                let ch = match i {
                    0..=7 => 149u16,
                    _ => 6u16,
                };
                chanseq_val.extend_from_slice(&ch.to_le_bytes());
            }
            let tlvs = vec![
                election_v2_tlv(50, 200),
                version_tlv(0x34, 1),
                awdl::Tlv { kind: 18, value: &chanseq_val },
            ];
            state.apply_action(peer_addr, mif(), &tlvs, 1_000);

            // filin should have adopted the master's sequence
            assert_eq!(state.channel_sequence[0], [149, 0]); // chan 149
            assert_eq!(state.channel_sequence[8], [6, 0]); // chan 6
            assert_ne!(
                state.channel_sequence[0],
                [44, state.channel_sequence[0][1]]
            );
        }

        #[test]
        fn reverts_to_static_anchor_sequence_when_master_lost() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];

            // adopt a master with a non-44 sequence
            let mut chanseq_val = vec![15, 3, 0, 3];
            chanseq_val.extend_from_slice(&0xffffu16.to_le_bytes());
            for _ in 0..16 {
                chanseq_val.extend_from_slice(&149u16.to_le_bytes());
            }
            let tlvs = vec![
                election_v2_tlv(50, 200),
                version_tlv(0x34, 1),
                awdl::Tlv { kind: 18, value: &chanseq_val },
            ];
            state.apply_action(peer_addr, mif(), &tlvs, 1_000);
            assert_eq!(state.channel_sequence[0][0], 149);

            // master times out → revert to anchor (44)
            state.clean_peers(1_000 + peers::PEER_TIMEOUT_US + 1);
            assert_eq!(state.channel_sequence[0][0], 44);
        }

        #[test]
        fn update_channel_detects_needed_switch() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            // current_channel starts at 0; desired is 44 (slot 0)
            let switch = state.update_channel(0);
            assert_eq!(switch, Some(44));
            assert_eq!(state.current_channel, 44);

            // calling again: no switch needed
            assert_eq!(state.update_channel(0), None);
        }

        #[test]
        fn desired_channel_reflects_sequence_slot() {
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            // put a null slot at position 1
            state.channel_sequence[1] = [0, 0];

            // slot 0 (t=0): channel 44
            assert_eq!(state.desired_channel(0), 44);
            // advance to EAW slot 1: 64 TU later = 65536 us
            assert_eq!(state.desired_channel(65536), 0);
        }
    }
}

pub mod runtime {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    use crate::{
        awdl, channel, ieee80211, iface::Config, os, pcap, radiotap, rx, sync::TsftBridge,
    };

    #[derive(Debug, PartialEq, Eq)]
    pub enum Error {
        Tap,
        Netdev,
        Packet,
        Channel,
        Poll,
        Io,
        Pcap,
    }

    pub struct Links {
        pub tap: File,
        pub packet: os::packet::PacketSocket,
        self_addr: [u8; 6],
        anchor_channel: u16,
        pcap: Option<pcap::Writer<File>>,
    }

    pub fn open_links(config: &Config) -> Result<Links, Error> {
        let tap = os::tun::open_tap(&config.host_iface).map_err(|_| Error::Tap)?;
        let monitor_addr =
            os::netdev::get_hwaddr(&config.monitor_iface).map_err(|_| Error::Netdev)?;
        os::netdev::set_hwaddr(&config.host_iface, monitor_addr).map_err(|_| Error::Netdev)?;
        os::netdev::set_mtu(&config.host_iface, 1450).map_err(|_| Error::Netdev)?;
        os::netdev::set_up(&config.host_iface).map_err(|_| Error::Netdev)?;
        let frequency_mhz =
            channel::channel_to_frequency_mhz(config.anchor_channel).ok_or(Error::Channel)?;
        os::nl80211::set_channel(&config.monitor_iface, u32::from(frequency_mhz))
            .map_err(|_| Error::Channel)?;
        let packet = os::packet::PacketSocket::open_bound(&config.monitor_iface)
            .map_err(|_| Error::Packet)?;
        let pcap = config
            .pcap_path
            .as_deref()
            .map(|path| {
                File::create(path)
                    .and_then(pcap::Writer::new)
                    .map_err(|_| Error::Pcap)
            })
            .transpose()?;

        Ok(Links {
            tap,
            packet,
            self_addr: monitor_addr,
            anchor_channel: config.anchor_channel,
            pcap,
        })
    }

    pub fn run(mut links: Links) -> Result<(), Error> {
        let mut bridge = TsftBridge::default();
        let mut awdl_state = crate::state::AwdlState::new(
            links.self_addr,
            hostname(),
            links.anchor_channel,
            host_time_us(),
        );
        let mut announce = AnnounceScheduler::new(host_time_us());
        let mut wlan_buf = vec![0u8; 4096];
        let mut tap_buf = vec![0u8; 2048];

        loop {
            let now_us = host_time_us();
            if announce.clean_due(now_us) {
                let removed = awdl_state.clean_peers(now_us);
                if !removed.is_empty() {
                    tracing::info!(removed = removed.len(), peers = awdl_state.peers.len(), "peer table cleaned");
                }
            }
            for subtype in announce.due(now_us, &awdl_state) {
                let seq = awdl_state.next_data_seq();
                let frame = build_announce_frame(subtype, &awdl_state, seq << 4, now_us);
                match links.packet.send(&frame) {
                    Ok(sent) => tracing::trace!(sent, ?subtype, "injected awdl announce frame"),
                    Err(err) => tracing::trace!(?err, ?subtype, "packet announce send failed"),
                }
            }

            let mut fds = [
                libc::pollfd {
                    fd: links.packet.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: links.tap.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let poll_timeout_ms = announce.poll_timeout_ms(host_time_us());
            // SAFETY: fds points to two valid pollfd entries for the duration of the call.
            let rc =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, poll_timeout_ms) };
            if rc < 0 {
                return Err(Error::Poll);
            }

            if fds[0].revents & libc::POLLIN != 0 {
                match links.packet.recv(&mut wlan_buf) {
                    Ok(len) => {
                        let now_us = host_time_us();
                        match rx::parse_action_frame(&wlan_buf[..len], now_us, &mut bridge) {
                            Ok(frame) => {
                                let src = frame.source;
                                let subtype = frame.action.subtype;
                                let rx_time_us = frame.rx_time_us;
                                let rssi_dbm = frame.rssi_dbm;
                                let tlv_count = frame.tlvs.len();
                                awdl_state.apply_action(src, subtype, &frame.tlvs, rx_time_us);
                                tracing::trace!(
                                    rx_time_us,
                                    rssi_dbm,
                                    tlv_count,
                                    master = ?awdl_state.election.master_addr,
                                    peers = awdl_state.peers.len(),
                                    "rx awdl action frame"
                                );
                            }
                            Err(err) => match decapsulate_rx_data(&wlan_buf[..len]) {
                                Ok((source, frame)) => {
                                    // owl only accepts data from a known peer
                                    // (owl/src/rx.c:361) to limit the attack surface.
                                    if awdl_state.peers.get(&source).is_none() {
                                        tracing::trace!(
                                            ?source,
                                            "drop data frame from non-peer"
                                        );
                                    } else {
                                        links
                                            .tap
                                            .write_all(&frame.bytes)
                                            .map_err(|_| Error::Io)?;
                                        tracing::trace!(
                                            len = frame.bytes.len(),
                                            "wrote ethernet frame to tap"
                                        );
                                    }
                                }
                                Err(data_err) => {
                                    if let Some(pcap) = links.pcap.as_mut() {
                                        if let Err(err) =
                                            pcap.write_packet(now_us, &wlan_buf[..len])
                                        {
                                            tracing::trace!(
                                                ?err,
                                                "pcap ignored-frame write failed"
                                            );
                                        }
                                    }
                                    let ieee80211_preview =
                                        ieee80211_payload_preview(&wlan_buf[..len]);
                                    tracing::trace!(
                                        ?err,
                                        ?data_err,
                                        %ieee80211_preview,
                                        "ignored wlan frame"
                                    );
                                }
                            },
                        }
                    }
                    Err(err) => tracing::trace!(?err, "packet recv failed"),
                }
            }

            if fds[1].revents & libc::POLLIN != 0 {
                let len = links.tap.read(&mut tap_buf).map_err(|_| Error::Io)?;
                if len >= 14 {
                    let dst = mac_from_slice(&tap_buf[0..6]);
                    let src = mac_from_slice(&tap_buf[6..12]);
                    match awdl::encapsulate_ethernet_payload(&tap_buf[..len], awdl_state.tx_seq) {
                        Ok(body) => {
                            let seq = awdl_state.next_data_seq();
                            let frame = radiotap_prefixed(ieee80211::build_data_frame(
                                dst,
                                src,
                                AWDL_BSSID,
                                seq << 4,
                                &body,
                            ));
                            match links.packet.send(&frame) {
                                Ok(sent) => tracing::trace!(sent, "injected awdl data frame"),
                                Err(err) => tracing::trace!(?err, "packet send failed"),
                            }
                        }
                        Err(err) => tracing::trace!(?err, len, "ignored host tap frame"),
                    }
                }
            }
        }
    }

    fn mac_from_slice(bytes: &[u8]) -> [u8; 6] {
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&bytes[..6]);
        mac
    }

    const TU_US: u64 = 1024;
    const PSF_INTERVAL_TU: u64 = 110;
    /// owl `PSF_INTERVAL_MASTER_TU` is 110; slave is 440. owl clean interval
    /// (`PEERS_DEFAULT_CLEAN_INTERVAL`) = 1 000 000 us (owl/src/peers.c:29).
    const CLEAN_INTERVAL_US: u64 = 1_000_000;
    const AWDL_BROADCAST: [u8; 6] = [0xff; 6];
    const AWDL_BSSID: [u8; 6] = [0x00, 0x25, 0x00, 0xff, 0x94, 0x73];

    #[derive(Debug, PartialEq, Eq)]
    struct AnnounceScheduler {
        next_psf_us: u64,
        next_mif_us: u64,
        next_clean_us: u64,
    }

    impl AnnounceScheduler {
        fn new(now_us: u64) -> Self {
            Self {
                next_psf_us: now_us + PSF_INTERVAL_TU * TU_US,
                // owl arms the MIF timer to fire immediately the first time
                // (core.c:419), then awdl_send_mif reschedules to mid-EAW.
                next_mif_us: now_us,
                next_clean_us: now_us + CLEAN_INTERVAL_US,
            }
        }

        fn poll_timeout_ms(&self, now_us: u64) -> i32 {
            let next = self.next_psf_us.min(self.next_mif_us).min(self.next_clean_us);
            let delta_us = next.saturating_sub(now_us);
            delta_us.div_ceil(1_000).min(i32::MAX as u64) as i32
        }

        /// owl `awdl_clean_peers` timer (owl/daemon/core.c:320-336) fires every
        /// `CLEAN_INTERVAL_US`.
        fn clean_due(&mut self, now_us: u64) -> bool {
            if now_us >= self.next_clean_us {
                while self.next_clean_us <= now_us {
                    self.next_clean_us += CLEAN_INTERVAL_US;
                }
                true
            } else {
                false
            }
        }

        /// Returns the action subtypes whose timers have fired, rescheduling
        /// each. MIF follows owl `awdl_send_mif` (owl/daemon/core.c:184-200): it
        /// only fires when our current channel slot is non-null and is rearmed
        /// to the middle of the next EAW. PSF fires every `PSF_INTERVAL_TU`
        /// (owl `awdl_send_psf`, core.c:178-182).
        fn due(
            &mut self,
            now_us: u64,
            awdl_state: &crate::state::AwdlState,
        ) -> Vec<awdl::ActionSubtype> {
            let mut due = Vec::new();
            if now_us >= self.next_mif_us {
                let slot =
                    usize::from(awdl_state.sync.current_eaw(now_us) % 16);
                let channel = crate::schedule::slot_channel(&awdl_state.channel_sequence, slot);
                if channel > 0 {
                    due.push(awdl::ActionSubtype::Mif);
                }
                self.next_mif_us = next_mif_us(&awdl_state.sync, now_us);
            }
            if now_us >= self.next_psf_us {
                due.push(awdl::ActionSubtype::Psf);
                while self.next_psf_us <= now_us {
                    self.next_psf_us += PSF_INTERVAL_TU * TU_US;
                }
            }
            due
        }
    }

    /// owl `awdl_send_mif` rearms to the middle of the next EAW:
    /// `next_aw + eaw_len/2` (owl/daemon/core.c:199).
    fn next_mif_us(sync: &crate::sync::SyncState, now_us: u64) -> u64 {
        let next_aw = sync.next_aw_us(now_us);
        let eaw_len = u64::from(sync.presence_mode) * u64::from(sync.aw_period_tu) * TU_US;
        now_us + next_aw + eaw_len / 2
    }

    fn build_announce_frame(
        subtype: awdl::ActionSubtype,
        awdl_state: &crate::state::AwdlState,
        sequence_control: u16,
        now_us: u64,
    ) -> Vec<u8> {
        let tlvs = build_announce_tlvs(subtype, awdl_state, now_us);
        let action = awdl::build_action_body(subtype, now_us as u32, now_us as u32, &tlvs);
        let frame = ieee80211::build_management_action_frame(
            AWDL_BROADCAST,
            awdl_state.self_addr,
            AWDL_BSSID,
            sequence_control,
            &action,
        );
        radiotap_prefixed(frame)
    }

    fn build_announce_tlvs(
        subtype: awdl::ActionSubtype,
        awdl_state: &crate::state::AwdlState,
        now_us: u64,
    ) -> Vec<u8> {
        let mut tlvs = Vec::new();
        push_tlv(
            &mut tlvs,
            4,
            &build_sync_parameters_tlv_value(awdl_state, now_us),
        );
        push_tlv(
            &mut tlvs,
            5,
            &build_election_parameters_tlv_value(&awdl_state.election),
        );
        push_tlv(
            &mut tlvs,
            18,
            &build_channel_sequence_tlv_value(&awdl_state.channel_sequence),
        );
        push_tlv(
            &mut tlvs,
            24,
            &build_election_parameters_v2_tlv_value(&awdl_state.election),
        );
        push_tlv(&mut tlvs, 6, &build_service_parameters_tlv_value());
        if subtype == awdl::ActionSubtype::Mif {
            push_tlv(&mut tlvs, 7, &build_ht_capabilities_tlv_value());
            push_tlv(&mut tlvs, 16, &build_arpa_tlv_value(&awdl_state.name));
        }
        push_tlv(
            &mut tlvs,
            12,
            &build_data_path_state_tlv_value(awdl_state.self_addr, awdl_state.anchor_channel),
        );
        push_tlv(&mut tlvs, 21, &build_version_tlv_value(awdl_state.version, awdl_state.dev_class));
        tlvs
    }

    fn radiotap_prefixed(mut frame: Vec<u8>) -> Vec<u8> {
        let mut prefixed = radiotap::build_tx_header();
        prefixed.append(&mut frame);
        prefixed
    }

    fn push_tlv(out: &mut Vec<u8>, kind: u8, value: &[u8]) {
        out.push(kind);
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value);
    }

    fn build_sync_parameters_tlv_value(
        awdl_state: &crate::state::AwdlState,
        now_us: u64,
    ) -> Vec<u8> {
        let sync = &awdl_state.sync;
        let next_aw_tu = sync.next_aw_tu(now_us);
        let aw_period = sync.aw_period_tu;
        let presence_mode = sync.presence_mode as u8;
        let eaw_len_tu = u16::from(presence_mode) * aw_period;
        let elapsed_in_eaw_tu = eaw_len_tu.saturating_sub(next_aw_tu);
        let remaining_aw_length = aw_period.saturating_sub(elapsed_in_eaw_tu);
        let channel = awdl_state.anchor_channel as u8;
        let mut value = Vec::with_capacity(73);
        value.push(channel); // next_aw_channel
        value.extend_from_slice(&next_aw_tu.to_le_bytes()); // tx_down_counter
        value.push(channel); // master_channel
        value.push(0); // guard_time
        value.extend_from_slice(&aw_period.to_le_bytes()); // aw_period
        value.extend_from_slice(&(PSF_INTERVAL_TU as u16).to_le_bytes()); // af_period
        value.extend_from_slice(&0x1800u16.to_le_bytes()); // flags
        value.extend_from_slice(&aw_period.to_le_bytes()); // aw_ext_length
        value.extend_from_slice(&aw_period.to_le_bytes()); // aw_com_length
        value.extend_from_slice(&remaining_aw_length.to_le_bytes());
        value.push(presence_mode - 1); // min_ext
        value.push(presence_mode - 1); // max_ext_multicast
        value.push(presence_mode - 1); // max_ext_unicast
        value.push(presence_mode - 1); // max_ext_af
        value.extend_from_slice(&awdl_state.election.master_addr); // master_addr
        value.push(presence_mode); // presence_mode
        value.push(0); // reserved
        value.extend_from_slice(&sync.current_aw(now_us).to_le_bytes()); // next_aw_seq
        value.extend_from_slice(&sync.current_aw(now_us).to_le_bytes()); // ap_alignment
        value.extend_from_slice(&build_channel_sequence_value(&awdl_state.channel_sequence));
        value.extend_from_slice(&[0, 0]); // padding
        value
    }

    fn build_channel_sequence_tlv_value(channel_sequence: &[[u8; 2]; 16]) -> Vec<u8> {
        let mut value = build_channel_sequence_value(channel_sequence);
        value.extend_from_slice(&[0, 0, 0]); // padding (owl awdl_init_chanseq_tlv)
        value
    }

    /// owl `awdl_init_chanseq` (owl/src/tx.c:80-96): count=15, encoding=3
    /// (OPCLASS), duplicate_count=0, step_count=3, fill=0xffff, then 16
    /// `(chan_num, opclass)` pairs.
    fn build_channel_sequence_value(channel_sequence: &[[u8; 2]; 16]) -> Vec<u8> {
        let mut value = Vec::with_capacity(38);
        value.extend_from_slice(&[15, 3, 0, 3]);
        value.extend_from_slice(&0xffffu16.to_le_bytes());
        for slot in channel_sequence.iter() {
            value.extend_from_slice(slot);
        }
        value
    }

    /// owl `awdl_init_election_params_tlv` (owl/src/tx.c:166-186): flags, id,
    /// distancetop(=height), unknown, master_addr, master_metric, self_metric.
    fn build_election_parameters_tlv_value(
        election: &crate::election::ElectionState,
    ) -> Vec<u8> {
        let mut value = Vec::with_capacity(21);
        value.push(0); // flags
        value.extend_from_slice(&0u16.to_le_bytes()); // id
        value.push(election.height as u8); // distancetop
        value.push(0); // unknown
        value.extend_from_slice(&election.master_addr);
        value.extend_from_slice(&election.master_metric.to_le_bytes());
        value.extend_from_slice(&election.self_metric.to_le_bytes());
        value.extend_from_slice(&[0, 0]); // pad
        value
    }

    /// owl `awdl_init_election_params_v2_tlv` (owl/src/tx.c:188-206).
    fn build_election_parameters_v2_tlv_value(
        election: &crate::election::ElectionState,
    ) -> Vec<u8> {
        let mut value = Vec::with_capacity(40);
        value.extend_from_slice(&election.master_addr);
        value.extend_from_slice(&election.sync_addr);
        value.extend_from_slice(&election.master_counter.to_le_bytes());
        value.extend_from_slice(&election.height.to_le_bytes()); // distance_to_master
        value.extend_from_slice(&election.master_metric.to_le_bytes());
        value.extend_from_slice(&election.self_metric.to_le_bytes());
        value.extend_from_slice(&0u32.to_le_bytes()); // unknown
        value.extend_from_slice(&0u32.to_le_bytes()); // reserved
        value.extend_from_slice(&election.self_counter.to_le_bytes());
        value
    }

    fn build_service_parameters_tlv_value() -> [u8; 9] {
        [0; 9]
    }

    fn build_ht_capabilities_tlv_value() -> [u8; 8] {
        [0x00, 0x00, 0xce, 0x11, 0x1b, 0xff, 0x00, 0x00]
    }

    fn build_arpa_tlv_value(name: &str) -> Vec<u8> {
        let mut value = Vec::with_capacity(2 + name.len() + 2);
        value.push(3); // flags
        value.push(name.len() as u8);
        value.extend_from_slice(name.as_bytes());
        value.extend_from_slice(&0xc00cu16.to_be_bytes()); // .local
        value
    }

    fn build_data_path_state_tlv_value(self_addr: [u8; 6], anchor_channel: u16) -> Vec<u8> {
        let mut value = Vec::with_capacity(15);
        value.extend_from_slice(&0x8f24u16.to_le_bytes());
        value.extend_from_slice(b"X0\0");
        value.extend_from_slice(&social_channel_bits(anchor_channel).to_le_bytes());
        value.extend_from_slice(&self_addr);
        value.extend_from_slice(&0u16.to_le_bytes());
        value
    }

    fn build_version_tlv_value(version: u8, devclass: u8) -> [u8; 2] {
        [version, devclass]
    }

    fn social_channel_bits(anchor_channel: u16) -> u16 {
        match anchor_channel {
            6 => 0x0001,
            44 => 0x0002,
            149 => 0x0004,
            _ => 0,
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum DataParseError {
        Radiotap(radiotap::RadiotapError),
        Fcs(rx::ParseError),
        Ieee80211(ieee80211::ParseError),
        Awdl(awdl::ParseError),
    }

    fn decapsulate_rx_data(frame: &[u8]) -> Result<([u8; 6], awdl::EthernetFrame), DataParseError> {
        let radiotap = radiotap::parse_header(frame).map_err(DataParseError::Radiotap)?;
        let ieee80211_payload =
            rx::ieee80211_payload_without_fcs(&radiotap).map_err(DataParseError::Fcs)?;
        let mac =
            ieee80211::parse_mac_header(ieee80211_payload).map_err(DataParseError::Ieee80211)?;
        let ethernet =
            awdl::decapsulate_data_frame(mac.body, mac.source, mac.destination)
                .map_err(DataParseError::Awdl)?;
        Ok((mac.source, ethernet))
    }

    fn ieee80211_payload_preview(frame: &[u8]) -> String {
        match radiotap::parse_header(frame) {
            Ok(header) => match rx::ieee80211_payload_without_fcs(&header) {
                Ok(payload) => hex_preview(payload, 64),
                Err(_) => hex_preview(header.payload, 64),
            },
            Err(_) => hex_preview(frame, 64),
        }
    }

    fn hex_preview(bytes: &[u8], limit: usize) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let len = bytes.len().min(limit);
        let mut out = String::with_capacity(len.saturating_mul(3).saturating_sub(1));
        for (index, byte) in bytes.iter().copied().take(limit).enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
        out
    }

    fn host_time_us() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: ts is valid writable memory for clock_gettime.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        if rc < 0 {
            return 0;
        }
        (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
    }

    /// owl `gethostname` for the ARPA TLV peer name (owl/daemon/core.c:362-364).
    fn hostname() -> String {
        let mut buf = [0u8; 256];
        // SAFETY: buf is valid writable memory for gethostname.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc < 0 {
            return String::from("filin");
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn scheduler_fires_psf_before_mif() {
            // MIF is armed immediately (now) then rescheduled to mid next-EAW.
            // PSF fires one PSF_INTERVAL (110 TU = 112640 us) after start.
            let state =
                crate::state::AwdlState::new([0x02, 1, 2, 3, 4, 5], "filin".into(), 44, 1_000_000);
            let mut scheduler = AnnounceScheduler::new(1_000_000);

            // MIF fires immediately; next MIF lands at mid next-EAW:
            //   next_aw(65536) + eaw_len/2(32768) = 98304 us after start.
            assert_eq!(scheduler.due(1_000_000, &state), vec![awdl::ActionSubtype::Mif]);
            assert!(scheduler.due(1_050_000, &state).is_empty());
            assert_eq!(scheduler.due(1_098_304, &state), vec![awdl::ActionSubtype::Mif]);
            // PSF lands at 1_000_000 + 110*1024 = 1_112_640.
            let due = scheduler.due(1_112_640, &state);
            assert!(due.contains(&awdl::ActionSubtype::Psf));
        }

        fn demo_state() -> crate::state::AwdlState {
            crate::state::AwdlState::new(
                [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee],
                "filin".into(),
                44,
                1_000_000,
            )
        }

        #[test]
        fn builds_broadcast_psf_announce_frame() {
            let state = demo_state();
            let frame = build_announce_frame(awdl::ActionSubtype::Psf, &state, 0x0120, 1_000_000);
            let radiotap = radiotap::parse_header(&frame).expect("valid tx radiotap header");
            assert_eq!(radiotap.header_len, 9);
            let mac = ieee80211::parse_mac_header(radiotap.payload).expect("valid action header");
            let action = awdl::parse_action_frame(mac.body).expect("valid awdl action");
            let tlvs = awdl::parse_tlvs(action.tlvs).expect("valid announce TLVs");

            assert_eq!(mac.frame_control, 0x00d0);
            assert_eq!(mac.destination, [0xff; 6]);
            assert_eq!(mac.source, [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee]);
            assert_eq!(mac.bssid, AWDL_BSSID);
            assert_eq!(mac.sequence_control, 0x0120);
            assert_eq!(action.subtype, awdl::ActionSubtype::Psf);
            assert!(tlvs.iter().any(|tlv| tlv.kind == 4));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 5));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 18));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 24));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 6));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 12));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 21));
            assert!(!tlvs.iter().any(|tlv| tlv.kind == 7));
            assert!(!tlvs.iter().any(|tlv| tlv.kind == 16));
            // chanseq TLV now uses owl OPCLASS encoding (3) with opclass byte 0x80.
            let chanseq = tlvs.iter().find(|t| t.kind == 18).unwrap().value;
            assert_eq!(chanseq[1], 3); // encoding = OPCLASS
            assert_eq!(&chanseq[6..8], &[44, 0x80]); // first slot (chan 44, opclass 0x80)
        }

        #[test]
        fn announce_election_tlvs_carry_real_state() {
            let mut state = demo_state();
            // adopt a peer as master so the election state is non-default
            let peer_addr = [0x02, 0, 0, 0, 0, 9];
            let mut v2 = vec![0u8; 40];
            v2[0..6].copy_from_slice(&peer_addr);
            v2[6..12].copy_from_slice(&peer_addr);
            v2[12..16].copy_from_slice(&77u32.to_le_bytes());
            v2[16..20].copy_from_slice(&1u32.to_le_bytes()); // distance -> height
            v2[20..24].copy_from_slice(&333u32.to_le_bytes());
            v2[24..28].copy_from_slice(&60u32.to_le_bytes());
            v2[36..40].copy_from_slice(&5u32.to_le_bytes());
            let version = awdl::Tlv { kind: 21, value: &[0x34, 1] };
            let v2_tlv = awdl::Tlv { kind: 24, value: &v2 };
            state.apply_action(peer_addr, awdl::ActionSubtype::Mif, &[version, v2_tlv], 1_100_000);

            let frame = build_announce_frame(awdl::ActionSubtype::Psf, &state, 0, 1_200_000);
            let radiotap = radiotap::parse_header(&frame).expect("valid tx radiotap");
            let mac = ieee80211::parse_mac_header(radiotap.payload).expect("valid action header");
            let action = awdl::parse_action_frame(mac.body).expect("valid awdl action");
            let tlvs = awdl::parse_tlvs(action.tlvs).expect("valid announce TLVs");

            // sync-params TLV master_addr must be the elected master, not self.
            let sync = tlvs.iter().find(|t| t.kind == 4).unwrap().value;
            assert_eq!(&sync[21..27], &peer_addr);
            // election v2 TLV reflects adopted master/counter/height.
            let ev2 = tlvs.iter().find(|t| t.kind == 24).unwrap().value;
            assert_eq!(&ev2[0..6], &peer_addr); // master_addr
            assert_eq!(&ev2[6..12], &peer_addr); // sync_addr
            assert_eq!(u32::from_le_bytes(ev2[12..16].try_into().unwrap()), 77); // master_counter
        }

        #[test]
        fn builds_mif_with_owl_extra_tlvs() {
            let state = demo_state();
            let frame = build_announce_frame(awdl::ActionSubtype::Mif, &state, 0x0120, 1_000_000);
            let radiotap = radiotap::parse_header(&frame).expect("valid tx radiotap header");
            let mac = ieee80211::parse_mac_header(radiotap.payload).expect("valid action header");
            let action = awdl::parse_action_frame(mac.body).expect("valid awdl action");
            let tlvs = awdl::parse_tlvs(action.tlvs).expect("valid announce TLVs");

            assert_eq!(action.subtype, awdl::ActionSubtype::Mif);
            assert!(tlvs.iter().any(|tlv| tlv.kind == 7));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 16));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 12));
            assert!(tlvs.iter().any(|tlv| tlv.kind == 21));
        }

        #[test]
        fn formats_hex_preview_with_limit() {
            assert_eq!(hex_preview(&[0xde, 0xad, 0xbe, 0xef], 3), "de ad be");
        }

        #[test]
        fn previews_payload_after_radiotap_header() {
            let mut frame = vec![
                0x00, 0x00, 0x08, 0x00, // radiotap version, pad, len
                0x00, 0x00, 0x00, 0x00, // no present fields
                0xd0, 0x00,
            ];
            frame.extend(0u8..80);

            assert_eq!(ieee80211_payload_preview(&frame).split(' ').count(), 64);
            assert!(ieee80211_payload_preview(&frame).starts_with("d0 00 00 01"));
        }
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
            Error::Io(
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO),
            )
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_tap_ifreq_for_awdl0_without_packet_info() {
                let plan = plan_tap_ifreq("awdl0").expect("valid tap name");

                assert_eq!(&plan.name[..5], b"awdl0");
                assert_eq!(plan.name[5], 0);
                assert_eq!(
                    plan.flags,
                    (libc::IFF_TAP | libc::IFF_NO_PI) as libc::c_short
                );
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
            Error::Io(
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO),
            )
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
            let fd =
                unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
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
            Error::Io(
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO),
            )
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn plans_if_name_and_up_flags() {
                let name = plan_if_name("awdl0").expect("valid if name");

                assert_eq!(&name.bytes[..5], b"awdl0");
                assert_eq!(name.bytes[5], 0);
                assert_eq!(
                    up_flags(0),
                    (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short
                );
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

        pub fn plan_set_channel(
            ifindex: libc::c_int,
            frequency_mhz: u32,
        ) -> Result<ChannelRequest, Error> {
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
            socket.request(
                family_id,
                libc::NLM_F_REQUEST as u16 | libc::NLM_F_ACK as u16,
                &payload,
            )?;
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

            fn request(
                &self,
                nlmsg_type: u16,
                flags: u16,
                payload: &[u8],
            ) -> Result<Vec<u8>, Error> {
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
                return if code == 0 {
                    Ok(Vec::new())
                } else {
                    Err(Error::Io(-code))
                };
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
            Error::Io(
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO),
            )
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
