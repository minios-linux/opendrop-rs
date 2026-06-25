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
        /// QoS control field for QoS data frames (owl reads it at offset 24,
        /// owl/src/rx.c:537). `None` for non-QoS frames.
        pub qos_control: Option<u16>,
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
            qos_control: if is_qos_data(frame_control) {
                Some(u16::from_le_bytes([frame[24], frame[25]]))
            } else {
                None
            },
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

    /// owl `IEEE80211_QOS_CTL_A_MSDU_PRESENT` (owl/src/ieee80211.h:125).
    pub const QOS_A_MSDU_PRESENT: u16 = 0x0080;

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

    /// owl `enum awdl_chan_encoding` (owl/src/channel.h:27-31): the
    /// per-entry byte layout of an AWDL channel-sequence TLV.
    pub const CHAN_ENC_SIMPLE: u8 = 0;
    pub const CHAN_ENC_LEGACY: u8 = 1;
    pub const CHAN_ENC_OPCLASS: u8 = 3;

    /// owl `awdl_chan_encoding_size` (owl/src/channel.c:69-79): the number
    /// of bytes per chanseq entry for a given encoding, or `None` for an
    /// unknown encoding. SIMPLE packs one `chan_num` byte; LEGACY and
    /// OPCLASS both use two bytes (with the channel number in different
    /// positions — see [`chan_num`]).
    pub fn chan_encoding_size(enc: u8) -> Option<usize> {
        match enc {
            CHAN_ENC_SIMPLE => Some(1),
            CHAN_ENC_LEGACY | CHAN_ENC_OPCLASS => Some(2),
            _ => None,
        }
    }

    /// owl `awdl_chan_num` (owl/src/channel.c:56-67): decode the real
    /// Wi-Fi channel number from a chanseq entry's raw bytes under the
    /// given encoding, or `None` for an unknown encoding.
    ///
    /// - SIMPLE (0): one byte, `chan_num = bytes[0]`.
    /// - LEGACY (1): two bytes laid out `[flags, chan_num]`, so the channel
    ///   number is the SECOND byte. filin previously read it as the first,
    ///   which yielded garbage channel numbers (e.g. 174/232/245) on live
    ///   Apple frames and made the radio hop to invalid channels.
    /// - OPCLASS (3): two bytes laid out `[chan_num, opclass]`, so the
    ///   channel number is the FIRST byte.
    pub fn chan_num(enc: u8, bytes: &[u8]) -> Option<u8> {
        match enc {
            CHAN_ENC_SIMPLE => bytes.first().copied(),
            CHAN_ENC_LEGACY => bytes.get(1).copied(),
            CHAN_ENC_OPCLASS => bytes.first().copied(),
            _ => None,
        }
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

        // owl awdl_chan_encoding_size (channel.c:69-79). filin previously
        // mapped encoding 1→1 byte and rejected encoding 0, both wrong; the
        // correct sizes are 0→1, 1→2, 3→2 (owl channel.h:27-31).
        let field_len = chan_encoding_size(encoding).ok_or(ParseError::UnexpectedValue)?;
        let mut offset = 6;
        let mut channels = Vec::with_capacity(16);
        for _ in 0..16 {
            let entry = value
                .get(offset..offset + field_len)
                .ok_or(ParseError::TooShort)?;
            // owl awdl_chan_num (channel.c:56-67): decode the REAL channel
            // number per encoding, not a raw little-endian u16 of both
            // bytes (which mixed flags/opclass into the channel and produced
            // invalid channels like 174/232/245 on the live wire).
            let channel = u16::from(chan_num(encoding, entry).ok_or(ParseError::UnexpectedValue)?);
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

    /// owl `awdl_rx_data_amsdu` (owl/src/rx.c:391-418): iterates A-MSDU
    /// subframes. Each subframe has a 14-byte Ethernet-like header
    /// (dst, src, length-be), then `length` bytes of LLC/SNAP+AWDL-data, then
    /// padding to a 4-byte boundary.
    pub fn decapsulate_amsdu_data_frames(body: &[u8]) -> Result<Vec<EthernetFrame>, ParseError> {
        let mut out = Vec::new();
        let mut offset = 0;
        while offset < body.len() {
            if offset + 14 > body.len() {
                return Err(ParseError::TooShort);
            }
            let dst: [u8; 6] = body[offset..offset + 6].try_into().unwrap();
            let src: [u8; 6] = body[offset + 6..offset + 12].try_into().unwrap();
            let sub_len = u16::from_be_bytes([body[offset + 12], body[offset + 13]]) as usize;
            offset += 14;
            if offset + sub_len > body.len() {
                return Err(ParseError::TooShort);
            }
            let sub_body = &body[offset..offset + sub_len];
            offset += sub_len;
            // strip padding to 4-byte boundary (owl/src/rx.c:413)
            if offset < body.len() {
                offset += (4 - ((14 + sub_len) % 4)) % 4;
            }
            match decapsulate_data_frame(sub_body, src, dst) {
                Ok(frame) => out.push(frame),
                Err(err) => return Err(err),
            }
        }
        Ok(out)
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
        fn rejects_unknown_chanseq_encoding() {
            // owl defines only encodings 0, 1, 3 (channel.h:27-31). The old
            // filin parser accepted a non-existent encoding 2; it must now be
            // rejected so a malformed/unknown TLV can't misdecode channels.
            let mut value = vec![15, 2, 0, 3, 0xff, 0xff];
            for channel in 1u16..=16 {
                value.extend_from_slice(&channel.to_le_bytes());
            }
            assert_eq!(
                parse_channel_sequence(&value).unwrap_err(),
                ParseError::UnexpectedValue
            );
        }

        /// Build a chanseq TLV body with a fixed 6-byte header and 16 entries
        /// laid out per `encoding`, using `entry_bytes(i)` for slot i.
        fn chanseq_tlv<F: Fn(usize) -> Vec<u8>>(encoding: u8, entry_bytes: F) -> Vec<u8> {
            let mut value = vec![15, encoding, 0, 3, 0xff, 0xff];
            for i in 0..16 {
                value.extend(entry_bytes(i));
            }
            value
        }

        #[test]
        fn parses_opclass_channel_sequence_tlv() {
            // AWDL_CHAN_ENC_OPCLASS (3): two bytes [chan_num, opclass].
            // Apple's default chanseq (owl awdl_chanseq_init) is 8x149 then
            // 8x6 — exactly the kind of sequence filin must hop through.
            let value = chanseq_tlv(CHAN_ENC_OPCLASS, |i| {
                let ch = if i < 8 { 149 } else { 6 };
                vec![ch, 0x80]
            });
            let parsed =
                parse_channel_sequence(&value).expect("valid OPCLASS channel sequence TLV");
            assert_eq!(parsed.encoding, CHAN_ENC_OPCLASS);
            // Real channel numbers, not the raw opclass-or-u16 garbage.
            assert_eq!(&parsed.channels[0..8], &[149; 8]);
            assert_eq!(&parsed.channels[8..16], &[6; 8]);
        }

        #[test]
        fn parses_legacy_channel_sequence_tlv() {
            // AWDL_CHAN_ENC_LEGACY (1): two bytes [flags, chan_num], so the
            // channel number is the SECOND byte. filin previously read the
            // first byte (flags) as the channel, which produced the live
            // garbage channels (174/232/245…). This is the regression test
            // for the live iPhone upload stall.
            let value = chanseq_tlv(CHAN_ENC_LEGACY, |i| {
                let ch = if i < 8 { 44 } else { 149 };
                // flags byte is non-zero to prove we do not return it.
                vec![0xa4, ch]
            });
            let parsed =
                parse_channel_sequence(&value).expect("valid LEGACY channel sequence TLV");
            assert_eq!(parsed.encoding, CHAN_ENC_LEGACY);
            assert_eq!(&parsed.channels[0..8], &[44; 8]);
            assert_eq!(&parsed.channels[8..16], &[149; 8]);
        }

        #[test]
        fn parses_simple_channel_sequence_tlv() {
            // AWDL_CHAN_ENC_SIMPLE (0): one byte = chan_num. filin previously
            // rejected encoding 0 outright; owl accepts it.
            let value = chanseq_tlv(CHAN_ENC_SIMPLE, |i| {
                vec![if i < 8 { 44u8 } else { 6u8 }]
            });
            let parsed =
                parse_channel_sequence(&value).expect("valid SIMPLE channel sequence TLV");
            assert_eq!(parsed.encoding, CHAN_ENC_SIMPLE);
            assert_eq!(&parsed.channels[0..8], &[44u16; 8]);
            assert_eq!(&parsed.channels[8..16], &[6u16; 8]);
        }

        #[test]
        fn chan_num_decodes_per_encoding() {
            // owl awdl_chan_num (channel.c:56-67) for each encoding.
            assert_eq!(chan_num(CHAN_ENC_SIMPLE, &[44]), Some(44));
            assert_eq!(chan_num(CHAN_ENC_LEGACY, &[0xa4, 44]), Some(44));
            assert_eq!(chan_num(CHAN_ENC_OPCLASS, &[44, 0x80]), Some(44));
            assert_eq!(chan_num(9, &[44]), None);
        }

        #[test]
        fn chan_encoding_size_matches_owl() {
            assert_eq!(chan_encoding_size(CHAN_ENC_SIMPLE), Some(1));
            assert_eq!(chan_encoding_size(CHAN_ENC_LEGACY), Some(2));
            assert_eq!(chan_encoding_size(CHAN_ENC_OPCLASS), Some(2));
            assert_eq!(chan_encoding_size(9), None);
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
        fn decapsulates_amsdu_with_two_subframes() {
            // build two LLC/SNAP+AWDL-data subframes
            let sub1 = vec![
                0xaa, 0xaa, 0x03, 0x00, 0x17, 0xf2, 0x08, 0x00, // LLC/SNAP
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // AWDL data header
                0x86, 0xdd, // ethertype
                0x60, 0x00, 0x00, 0x00, // payload
            ];
            let sub2 = vec![
                0xaa, 0xaa, 0x03, 0x00, 0x17, 0xf2, 0x08, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x86, 0xdd,
                0x60, 0x00, 0x00, 0x01,
            ];
            let dst1 = [0x33, 0x33, 0x00, 0x00, 0x00, 0x01u8];
            let src1 = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
            let dst2 = [0x33, 0x33, 0x00, 0x00, 0x00, 0x02];
            let src2 = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xff];

            // A-MSDU body = subframe1-header + sub1 + padding + subframe2-header + sub2
            let mut body = Vec::new();
            body.extend_from_slice(&dst1);
            body.extend_from_slice(&src1);
            body.extend_from_slice(&(sub1.len() as u16).to_be_bytes());
            body.extend_from_slice(&sub1);
            // pad to 4-byte boundary: (14 + sub1.len()) % 4
            let pad1 = (4 - ((14 + sub1.len()) % 4)) % 4;
            body.extend(std::iter::repeat_n(0u8, pad1));
            body.extend_from_slice(&dst2);
            body.extend_from_slice(&src2);
            body.extend_from_slice(&(sub2.len() as u16).to_be_bytes());
            body.extend_from_slice(&sub2);

            let frames = decapsulate_amsdu_data_frames(&body).expect("valid A-MSDU");
            assert_eq!(frames.len(), 2);
            assert_eq!(&frames[0].bytes[0..6], &dst1);
            assert_eq!(&frames[0].bytes[6..12], &src1);
            assert_eq!(&frames[1].bytes[0..6], &dst2);
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
    /// owl `AWDL_UNICAST_GUARD_TU` (owl/src/schedule.h:28).
    pub const UNICAST_GUARD_TU: u16 = 3;

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

    /// owl `awdl_same_channel_as_peer` (owl/src/schedule.c:30-41): true if we
    /// and the peer are on the same non-zero channel at the current EAW slot.
    /// When filin has adopted the master's sequence, this is always true for
    /// the master; it may differ for non-master peers.
    pub fn same_channel_as_peer(
        own_seq: &[[u8; 2]; CHANSEQ_LEN],
        peer_seq: &[[u8; 2]; CHANSEQ_LEN],
        sync: &SyncState,
        now_us: u64,
    ) -> bool {
        let own_slot = usize::from(sync.current_eaw(now_us) % CHANSEQ_LEN as u16);
        let own_chan = slot_channel(own_seq, own_slot);
        let peer_chan = slot_channel(peer_seq, own_slot);
        own_chan != 0 && own_chan == peer_chan
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

    /// Decision returned by [`data_send_decision`]: whether the TX path
    /// should inject a data frame now (`Send`) or hold it for retry (`Hold`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DataSendDecision {
        /// Inject the frame now on the current channel/availability window.
        Send,
        /// Hold the frame and retry on the next availability window.
        Hold,
    }

    /// Pure TX-gate decision that ports owl's split between
    /// `awdl_send_unicast` (owl/daemon/core.c:202-244, per-peer and
    /// channel-gated) and `awdl_send_multicast` (owl/daemon/core.c:246-277,
    /// sent on the current AW channel with no per-peer gate).
    ///
    /// Given a data frame's destination MAC, the current channel, our own
    /// channel sequence, the peer table, and the current sync/now, decide
    /// whether the runtime TX path should inject the frame now.
    ///
    /// - **Multicast/broadcast** destinations — the Ethernet I/G bit is set,
    ///   `dst[0] & 0x01 == 1`, which covers IPv6 multicast `33:33:..` (mDNS
    ///   announce `ff02::fb`, NDP solicitations `33:33:ff:..`) and broadcast
    ///   `ff:ff:..` — are NOT peer-gated. Like owl's `awdl_send_multicast`,
    ///   they inject on the current channel during the multicast availability
    ///   window (slots 0 and 10) outside the multicast guard interval,
    ///   regardless of whether any single peer happens to be on this channel.
    ///   A multicast frame has no single destination peer, so the
    ///   "same channel as peer" lookup must not apply.
    /// - **Unicast** destinations keep owl's `awdl_send_unicast` gate: sent
    ///   only when the destination peer is known and on the same channel this
    ///   EAW, outside the unicast guard interval.
    pub fn data_send_decision(
        dst: &[u8; 6],
        current_channel: u8,
        own_seq: &[[u8; 2]; CHANSEQ_LEN],
        peers: &crate::peers::PeerTable,
        sync: &SyncState,
        now_us: u64,
    ) -> DataSendDecision {
        if current_channel == 0 {
            return DataSendDecision::Hold; // null slot: not available
        }
        if dst[0] & 0x01 == 1 {
            // owl awdl_send_multicast (core.c:254-255): multicast EAW +
            // multicast guard, never the per-peer same-channel gate.
            if !is_multicast_eaw(sync, now_us) {
                return DataSendDecision::Hold;
            }
            if can_send_in(sync, now_us, MULTICAST_GUARD_TU) != 0 {
                return DataSendDecision::Hold;
            }
            DataSendDecision::Send
        } else {
            // owl awdl_send_unicast (core.c:222-223): per-peer gate.
            let Some(peer) = peers.get(dst) else {
                return DataSendDecision::Hold; // non-peer unicast: drop/hold
            };
            if !same_channel_as_peer(own_seq, &peer.sequence, sync, now_us) {
                return DataSendDecision::Hold; // peer is on a different channel
            }
            if can_send_in(sync, now_us, UNICAST_GUARD_TU) != 0 {
                return DataSendDecision::Hold; // inside unicast guard interval
            }
            DataSendDecision::Send
        }
    }

    /// Maximum number of pending multicast frames buffered in the spread
    /// queue. Bounds memory if the host TAP feeds multicast faster than one
    /// hop cycle can drain it.
    pub const MULTICAST_SPREAD_CAP: usize = 32;

    /// One full hop cycle in microseconds: 16 EAW slots * 64 TU per EAW
    /// (presence_mode 4 * aw_period 16 TU) * 1024 us/TU. Pending multicast
    /// frames older than this are aged out so a frame whose remaining
    /// channels never recur (e.g. the master sequence changed mid-cycle)
    /// cannot leak.
    pub const HOP_CYCLE_US: u64 = 16 * 64 * TU_US;

    /// The distinct non-zero channel numbers in a channel sequence, deduped
    /// and sorted ascending. These are the channels a multicast/broadcast
    /// frame must be transmitted on so that a peer listening on ANY slot of
    /// the adopted master sequence hears it during one hop cycle. If the
    /// sequence contains no non-zero channels (filin has not adopted a
    /// master, or all slots are null), falls back to `[anchor]` — i.e. the
    /// pre-spread single-channel behaviour.
    pub fn distinct_channels(chanseq: &[[u8; 2]; CHANSEQ_LEN], anchor: u8) -> Vec<u8> {
        use std::collections::BTreeSet;
        let mut set: BTreeSet<u8> = BTreeSet::new();
        for slot in chanseq.iter() {
            if slot[0] != 0 {
                set.insert(slot[0]);
            }
        }
        if set.is_empty() {
            if anchor != 0 {
                vec![anchor]
            } else {
                Vec::new()
            }
        } else {
            set.into_iter().collect()
        }
    }

    /// AWDL social channels (owl `CHAN_OPCLASS_*`, owl/src/channel.h:51-53)
    /// that Apple devices scan on for AirDrop discovery. filin must visit
    /// ALL of these during its hop cycle — if the committed master only
    /// advertises a subset (e.g. a master in an active data session may use
    /// only 44 with some 6), an iPhone scanning on 149 will never discover
    /// filin.
    pub const AWDL_SOCIAL_CHANNELS: [(u8, u8); 3] = [
        (6, 0x51),    // CHAN_OPCLASS_6
        (44, 0x80),   // CHAN_OPCLASS_44
        (149, 0x80),  // CHAN_OPCLASS_149
    ];

    /// Ensure every AWDL social channel (6, 44, 149) appears at least once
    /// in the channel sequence by injecting any missing social channels into
    /// null or duplicate slots. This guarantees filin physically visits 149
    /// (where iPhones scan for AirDrop) even when the adopted master's
    /// sequence omits it.
    ///
    /// Slot selection: prefer null slots (channel 0) so we don't remove any
    /// channel the master advertised. If there are no nulls, overwrite a
    /// slot whose channel appears in other slots (a duplicate), preserving
    /// at least one occurrence of every distinct channel. Returns the
    /// channels that were injected (empty if the sequence already covered
    /// all social channels).
    ///
    /// After guaranteeing one slot per social channel, any REMAINING null
    /// slots are filled round-robin with the social channels (Part B: denser
    /// dwell) so filin spends several slots on 149 — not just one — and
    /// overlaps the iPhone's 149 listen windows more often. The round-robin
    /// starts at 149 to maximise 149 dwell first.
    pub fn ensure_social_coverage(seq: &mut [[u8; 2]; CHANSEQ_LEN]) -> Vec<u8> {
        let mut injected = Vec::new();
        for &(ch, opclass) in &AWDL_SOCIAL_CHANNELS {
            if seq.iter().any(|s| s[0] == ch) {
                continue; // already present
            }
            if let Some(i) = find_injectable_slot(seq) {
                seq[i] = [ch, opclass];
                injected.push(ch);
            }
        }
        // Part B: fill any remaining null slots round-robin with the social
        // channels (starting at 149) so filin dwells on 149 in several slots,
        // not just one. Only nulls are filled — master-advertised channels
        // are left untouched.
        fill_nulls_round_robin(seq);
        injected
    }

    /// Fill remaining null (channel-0) slots round-robin with the AWDL social
    /// channels, starting at 149 so 149 gets the first (and thus most) slots.
    /// Non-null slots are never touched, so the master's real channels stay
    /// intact.
    fn fill_nulls_round_robin(seq: &mut [[u8; 2]; CHANSEQ_LEN]) {
        // Round-robin order starting at 149 for maximum 149 dwell.
        let order = [(149u8, 0x80u8), (6, 0x51), (44, 0x80)];
        let mut idx = 0;
        for slot in seq.iter_mut() {
            if slot[0] == 0 {
                let (ch, opclass) = order[idx % order.len()];
                *slot = [ch, opclass];
                idx += 1;
            }
        }
    }

    /// Find a slot suitable for social-channel injection: a null slot, or
    /// failing that, a slot whose channel appears in at least one other slot
    /// (a duplicate we can safely overwrite).
    fn find_injectable_slot(seq: &[[u8; 2]; CHANSEQ_LEN]) -> Option<usize> {
        // Prefer a null slot.
        if let Some(i) = seq.iter().position(|s| s[0] == 0) {
            return Some(i);
        }
        // Else find a duplicate slot (channel appears more than once).
        for i in 0..CHANSEQ_LEN {
            let ch = seq[i][0];
            if seq.iter().filter(|s| s[0] == ch).count() > 1 {
                return Some(i);
            }
        }
        None
    }

    /// A multicast/broadcast frame pending transmission on the remaining
    /// channels of the master hop cycle.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PendingMulticast {
        /// Radiotap-prefixed IEEE 802.11 data frame bytes to inject.
        pub frame: Vec<u8>,
        /// Distinct channels this frame has NOT yet been transmitted on.
        pub channels_todo: std::collections::BTreeSet<u8>,
        /// Host time when enqueued, for TTL age-out.
        pub enqueued_at_us: u64,
    }

    /// Bounded FIFO queue of multicast frames being spread across the hop
    /// cycle.
    ///
    /// Ports owl's intent from `awdl_send_multicast` (owl/daemon/core.c:
    /// 246-277) but extends it: instead of emitting a multicast frame once
    /// on the current channel, each enqueued frame is transmitted on every
    /// distinct channel of the adopted master channel sequence over the next
    /// hop cycle, so a peer listening on any of those channels (e.g.
    /// 6/44/149) hears it.
    ///
    /// The queue is bounded (`cap`); enqueuing past the cap evicts the oldest
    /// entry so a host-TAP flood cannot grow it unboundedly. Entries older
    /// than `ttl_us` (one hop cycle) are aged out during [`Self::drain_for_channel`]
    /// so a frame whose remaining channels never recur does not leak.
    #[derive(Debug)]
    pub struct MulticastSpread {
        entries: std::collections::VecDeque<PendingMulticast>,
        cap: usize,
        ttl_us: u64,
    }

    impl MulticastSpread {
        pub fn new(cap: usize, ttl_us: u64) -> Self {
            Self {
                entries: std::collections::VecDeque::new(),
                cap,
                ttl_us,
            }
        }

        pub fn len(&self) -> usize {
            self.entries.len()
        }

        pub fn is_empty(&self) -> bool {
            self.entries.is_empty()
        }

        /// Enqueue a frame to be spread across the distinct channels of
        /// `chanseq` (falling back to `[anchor]` per [`distinct_channels`]).
        /// Returns `true` iff the frame was enqueued (there was at least one
        /// target channel). If the queue is full, the oldest entry is evicted
        /// first so the most recent announce always gets its hop cycle.
        pub fn enqueue(
            &mut self,
            frame: Vec<u8>,
            chanseq: &[[u8; 2]; CHANSEQ_LEN],
            anchor: u8,
            now_us: u64,
        ) -> bool {
            let channels: std::collections::BTreeSet<u8> =
                distinct_channels(chanseq, anchor).into_iter().collect();
            if channels.is_empty() {
                return false;
            }
            while self.entries.len() >= self.cap {
                self.entries.pop_front();
            }
            self.entries.push_back(PendingMulticast {
                frame,
                channels_todo: channels,
                enqueued_at_us: now_us,
            });
            true
        }

        /// Drain frames that should be transmitted on `channel` now. For each
        /// pending frame whose remaining-channel set contains `channel`,
        /// returns its frame bytes and removes `channel` from its todo set;
        /// frames whose todo set is then empty are dropped (fully spread).
        /// Entries older than `ttl_us` are dropped without being returned
        /// (aged out). Returns frame bytes to inject, in FIFO order.
        pub fn drain_for_channel(&mut self, channel: u8, now_us: u64) -> Vec<Vec<u8>> {
            let ttl = self.ttl_us;
            let mut out: Vec<Vec<u8>> = Vec::new();
            let mut keep = std::collections::VecDeque::with_capacity(self.entries.len());
            while let Some(mut entry) = self.entries.pop_front() {
                if now_us.saturating_sub(entry.enqueued_at_us) > ttl {
                    continue; // aged out: drop without sending
                }
                if entry.channels_todo.remove(&channel) {
                    // Clone: the entry stays queued for its remaining channels,
                    // so its frame bytes must survive until the todo set empties.
                    out.push(entry.frame.clone());
                    if !entry.channels_todo.is_empty() {
                        keep.push_back(entry);
                    }
                } else {
                    keep.push_back(entry);
                }
            }
            self.entries = keep;
            out
        }
    }

    // --- Part A: cached mDNS announce re-broadcast -------------------------

    /// Minimum dwell between re-broadcasts of the cached announce on the
    /// SAME social channel (~1 s). Prevents flooding while ensuring every
    /// ~1 s of dwell on 149 the iPhone gets a fresh announce.
    pub const ANNOUNCE_REBROADCAST_MIN_GAP_US: u64 = 1_000_000;

    /// How long the cached announce stays valid after the last refresh from
    /// the TAP. A few × the luftlift re-announce interval (~10 s): if no new
    /// multicast arrives for 45 s, stop advertising a (probably dead)
    /// receiver.
    pub const ANNOUNCE_CACHE_TTL_US: u64 = 45_000_000;

    /// Cache of the latest multicast mDNS announce read from the awdl0 TAP,
    /// re-broadcast on every social-channel visit so discovery does not
    /// depend on luftlift's ~10 s cadence aligning with a rare 149 hop.
    ///
    /// At most one frame is cached at a time (the latest); it is refreshed
    /// whenever a new multicast frame arrives and expires if no refresh
    /// arrives for [`ANNOUNCE_CACHE_TTL_US`]. Re-broadcasts are rate-limited
    /// per-channel to [`ANNOUNCE_REBROADCAST_MIN_GAP_US`].
    #[derive(Debug)]
    pub struct AnnounceCache {
        frame: Option<Vec<u8>>,
        cached_at_us: u64,
        /// Host time of the last re-broadcast per channel, for rate limiting.
        last_sent: std::collections::BTreeMap<u8, u64>,
        min_gap_us: u64,
        ttl_us: u64,
    }

    impl AnnounceCache {
        pub fn new(min_gap_us: u64, ttl_us: u64) -> Self {
            Self {
                frame: None,
                cached_at_us: 0,
                last_sent: std::collections::BTreeMap::new(),
                min_gap_us,
                ttl_us,
            }
        }

        /// Refresh the cache with a new multicast frame from the TAP.
        /// Always keeps just the latest frame (bounded).
        pub fn refresh(&mut self, frame: Vec<u8>, now_us: u64) {
            self.frame = Some(frame);
            self.cached_at_us = now_us;
        }

        /// Returns the cached frame to re-broadcast on `channel` now, iff:
        /// the cache is live (not expired), the channel is a social channel,
        /// and we have not already re-broadcast on this channel within the
        /// rate-limit window. Updates the per-channel rate-limit clock on a
        /// hit. Returns `None` if suppressed/expired/empty.
        pub fn rebroadcast(&mut self, channel: u8, now_us: u64) -> Option<&[u8]> {
            // Expire stale cache so a dead receiver stops being advertised.
            // Done before borrowing self.frame below to avoid a borrow conflict.
            if self.frame.is_some()
                && now_us.saturating_sub(self.cached_at_us) > self.ttl_us
            {
                self.frame = None;
            }
            let frame = self.frame.as_deref()?;
            if !AWDL_SOCIAL_CHANNELS.iter().any(|(c, _)| *c == channel) {
                return None; // re-broadcast only on social channels
            }
            if let Some(&last) = self.last_sent.get(&channel) {
                if now_us.saturating_sub(last) < self.min_gap_us {
                    return None; // rate-limited on this channel
                }
            }
            self.last_sent.insert(channel, now_us);
            Some(frame)
        }

        /// Whether the cache currently holds a non-expired frame.
        pub fn is_live(&self, now_us: u64) -> bool {
            self.frame
                .is_some()
                && now_us.saturating_sub(self.cached_at_us) <= self.ttl_us
        }
    }

    // --- Bug C: sticky transfer channel -------------------------------------

    /// Idle window after the last received unicast data frame from a peer
    /// during which filin keeps the radio PINNED to that peer's channel and
    /// suppresses channel hopping + master re-adoption. A bulk AirDrop
    /// /Upload is a sustained TCP/TLS stream: it cannot survive filin
    /// leaving the channel mid-body. Two EAWs (~128 ms) of silence is long
    /// enough to ride gaps between TCP segments while releasing quickly once
    /// the transfer truly ends.
    pub const TRANSFER_IDLE_TIMEOUT_US: u64 = 2 * 64 * TU_US;

    /// Record of the most recent unicast data RX used to pin the radio to
    /// the active-transfer peer's channel (Bug C).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TransferPin {
        pub peer: [u8; 6],
        pub channel: u8,
        pub last_rx_us: u64,
    }

    /// Pure decision for the sticky-transfer pin (Bug C): returns the
    /// `(peer, channel)` filin must hold while a unicast data flow is
    /// active, or `None` once the idle timeout has elapsed (transfer
    /// complete / peer went quiet). A pin whose recorded channel is 0 is
    /// treated as inactive. Used both to override [`desired_channel`] and to
    /// suppress master re-adoption ([`master_adoption_decision`]).
    pub fn transfer_active(
        pin: Option<&TransferPin>,
        now_us: u64,
        idle_timeout_us: u64,
    ) -> Option<([u8; 6], u8)> {
        let pin = pin?;
        if pin.channel == 0 {
            return None;
        }
        if now_us.saturating_sub(pin.last_rx_us) > idle_timeout_us {
            return None;
        }
        Some((pin.peer, pin.channel))
    }

    // --- Bug B: master-adoption hysteresis ----------------------------------

    /// How long a challenger must persistently win the election before filin
    /// commits to adopting its channel sequence. Stops the master thrash
    /// seen on a contended link (re-adopting ~44×/min flip-flopping between
    /// Apple devices). Two EAWs mirrors the transfer pin window so a single
    /// stray challenger frame cannot deschedule an in-flight transfer.
    pub const MASTER_DEBOUNCE_US: u64 = 2 * 64 * TU_US;

    /// Result of [`master_adoption_decision`]: which master to commit to now.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MasterAdoption {
        /// Keep the currently-committed master (challenger has not yet won
        /// the debounce window).
        Keep,
        /// Commit to the new election winner.
        Adopt,
    }

    /// Pure master-adoption hysteresis decision (Bug B). Given the
    /// currently-committed master, the latest election `winner`, the
    /// pending challenger and how long it has been winning, returns whether
    /// to commit to the winner now (`Adopt`) or hold the current master
    /// (`Keep`).
    ///
    /// - If the winner is already the committed master, returns `Adopt`
    ///   (no-op refresh; clears the pending challenger).
    /// - If the winner is the pending challenger AND it has been winning
    ///   for at least `debounce_us`, returns `Adopt`.
    /// - Otherwise returns `Keep` — the challenger must persist.
    pub fn master_adoption_decision(
        committed_master: [u8; 6],
        winner: [u8; 6],
        pending_master: [u8; 6],
        pending_since_us: u64,
        now_us: u64,
        debounce_us: u64,
    ) -> MasterAdoption {
        if winner == committed_master {
            return MasterAdoption::Adopt;
        }
        if winner == pending_master
            && now_us.saturating_sub(pending_since_us) >= debounce_us
        {
            return MasterAdoption::Adopt;
        }
        MasterAdoption::Keep
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

        #[test]
        fn same_channel_when_sequences_match() {
            let sync = synced();
            let seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert!(same_channel_as_peer(&seq, &seq, &sync, 1_000_000));
        }

        #[test]
        fn different_channel_when_sequences_diverge() {
            let sync = synced();
            let own = [[44u8, 0x80u8]; CHANSEQ_LEN];
            let peer = {
                let mut s = [[149u8, 0x80u8]; CHANSEQ_LEN];
                s[0] = [0, 0]; // slot 0 is null for peer
                s
            };
            // slot 0: own is 44, peer is 0 (null) → not same
            assert!(!same_channel_as_peer(&own, &peer, &sync, 1_000_000));
        }

        // --- data_send_decision: the multicast/broadcast TX fix ---

        fn empty_peers() -> crate::peers::PeerTable {
            crate::peers::PeerTable::new()
        }

        fn peer_on_channel(addr: [u8; 6], chan: u8) -> crate::peers::PeerTable {
            // Build a peer table whose single peer advertises `chan` on every
            // slot of its channel sequence (so same_channel_as_peer is true
            // iff our own sequence also has `chan` on the current slot).
            let mut table = crate::peers::PeerTable::new();
            table.touch(addr, 1_000_000);
            table.get_mut(&addr).unwrap().sequence = [[chan, 0x80]; CHANSEQ_LEN];
            table
        }

        /// The mDNS announce multicast destination (ff02::fb maps to the
        /// Ethernet multicast address 33:33:00:00:00:fb). Its first octet has
        /// the I/G bit set.
        const MDNS_V6_MCAST: [u8; 6] = [0x33, 0x33, 0x00, 0x00, 0x00, 0xfb];

        #[test]
        fn multicast_dst_is_not_peer_gated() {
            // Regression: an mDNS announce (multicast) must be injected on
            // the current channel during a multicast EAW even when the peer
            // table is empty (no single destination peer to channel-match).
            // owl `awdl_send_multicast` (core.c:246-277).
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            // 1_032_768 is ~32 TU into slot 0 (multicast EAW), clear of the
            // 16 TU multicast guard on both sides.
            assert_eq!(
                data_send_decision(
                    &MDNS_V6_MCAST,
                    44,
                    &own_seq,
                    &empty_peers(),
                    &sync,
                    1_032_768,
                ),
                DataSendDecision::Send,
            );
        }

        #[test]
        fn broadcast_dst_is_not_peer_gated() {
            // ff:ff:ff:ff:ff:ff also has the I/G bit set and must not be
            // held waiting for a per-peer channel match.
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert_eq!(
                data_send_decision(
                    &[0xff; 6],
                    44,
                    &own_seq,
                    &empty_peers(),
                    &sync,
                    1_032_768,
                ),
                DataSendDecision::Send,
            );
        }

        #[test]
        fn multicast_dst_held_off_multicast_eaw() {
            // Multicast frames are sent during multicast EAWs (slots 0/10)
            // like owl; mid-EAW slot 1 is not a multicast EAW, so hold.
            // 1_065_536 is the start of EAW slot 1 (one full EAW after
            // 1_000_000).
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert_eq!(
                data_send_decision(
                    &MDNS_V6_MCAST,
                    44,
                    &own_seq,
                    &empty_peers(),
                    &sync,
                    1_098_304,
                ),
                DataSendDecision::Hold,
            );
        }

        #[test]
        fn multicast_dst_held_on_null_channel() {
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert_eq!(
                data_send_decision(
                    &MDNS_V6_MCAST,
                    0,
                    &own_seq,
                    &empty_peers(),
                    &sync,
                    1_032_768,
                ),
                DataSendDecision::Hold,
            );
        }

        #[test]
        fn unicast_dst_with_no_peer_is_held() {
            // owl `awdl_send_unicast` drops/holds unicast to non-peers.
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            assert_eq!(
                data_send_decision(
                    &[0x02, 0, 0, 0, 0, 0x07],
                    44,
                    &own_seq,
                    &empty_peers(),
                    &sync,
                    1_032_768,
                ),
                DataSendDecision::Hold,
            );
        }

        #[test]
        fn unicast_dst_same_channel_as_peer_is_sent() {
            // Unicast keeps the per-peer same-channel gate: when the peer is
            // known and on our channel, send.
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            let peer_addr = [0x02, 0, 0, 0, 0, 0x07];
            let peers = peer_on_channel(peer_addr, 44);
            assert_eq!(
                data_send_decision(
                    &peer_addr,
                    44,
                    &own_seq,
                    &peers,
                    &sync,
                    1_032_768,
                ),
                DataSendDecision::Send,
            );
        }

        #[test]
        fn unicast_dst_different_channel_is_held() {
            // The unicast same-channel gate is preserved: peer on a
            // different channel this EAW → hold (frame buffered for retry).
            let sync = synced();
            let own_seq = [[44u8, 0x80u8]; CHANSEQ_LEN];
            let peer_addr = [0x02, 0, 0, 0, 0, 0x07];
            // Peer sequence has a null slot 0, so it is not on our channel
            // (44) during slot 0.
            let mut peers = crate::peers::PeerTable::new();
            peers.touch(peer_addr, 1_000_000);
            {
                let seq = &mut peers.get_mut(&peer_addr).unwrap().sequence;
                seq[0] = [0, 0];
            }
            assert_eq!(
                data_send_decision(&peer_addr, 44, &own_seq, &peers, &sync, 1_032_768),
                DataSendDecision::Hold,
            );
        }

        // --- MulticastSpread: spread mDNS announce across the hop cycle ---

        /// Build a 16-slot channel sequence from a slice of (chan, opclass)
        /// pairs, repeating the last pair to fill the remaining slots (so a
        /// short master chanseq like [6,44,149] fills the 16-slot array).
        fn chanseq(pairs: &[(u8, u8)]) -> [[u8; 2]; CHANSEQ_LEN] {
            let mut seq = [[0u8, 0u8]; CHANSEQ_LEN];
            let last = *pairs.last().unwrap_or(&(0, 0));
            for (i, p) in pairs.iter().enumerate() {
                seq[i] = [p.0, 0];
            }
            for slot in seq.iter_mut().take(CHANSEQ_LEN).skip(pairs.len()) {
                slot[0] = last.0;
            }
            seq
        }

        #[test]
        fn distinct_channels_dedupes_master_sequence() {
            // Master chanseq hops 6 / 44 / 149 (and repeats). Distinct set is
            // [6, 44, 149] — the channels a multicast announce must cover.
            let seq = chanseq(&[(6, 0), (44, 0), (149, 0)]);
            assert_eq!(distinct_channels(&seq, 44), vec![6, 44, 149]);
        }

        #[test]
        fn distinct_channels_ignores_null_slots() {
            // Null (0) slots are skipped, not treated as channel 0.
            let seq = chanseq(&[(6, 0), (0, 0), (149, 0)]);
            assert_eq!(distinct_channels(&seq, 44), vec![6, 149]);
        }

        #[test]
        fn distinct_channels_falls_back_to_anchor_when_no_master() {
            // No master adopted: all-zero sequence → single anchor channel,
            // i.e. the pre-spread single-channel behaviour.
            let seq = [[0u8, 0u8]; CHANSEQ_LEN];
            assert_eq!(distinct_channels(&seq, 44), vec![44]);
        }

        #[test]
        fn spread_sends_on_every_distinct_channel_then_drops() {
            // Core regression: one mDNS announce enqueued while the master
            // chanseq = [6,44,149] is sent exactly once on each of the three
            // distinct channels as filin hops through them, then dropped.
            let mut spread = MulticastSpread::new(MULTICAST_SPREAD_CAP, HOP_CYCLE_US);
            let seq = chanseq(&[(6, 0), (44, 0), (149, 0)]);
            assert!(spread.enqueue(vec![0xAA], &seq, 44, 1_000_000));
            assert_eq!(spread.len(), 1);

            // Hop to ch6 → inject, still pending (44/149 left).
            assert_eq!(spread.drain_for_channel(6, 1_000_000), vec![vec![0xAA]]);
            assert_eq!(spread.len(), 1);
            // Hop to ch44 → inject, still pending (149 left).
            assert_eq!(spread.drain_for_channel(44, 1_000_000), vec![vec![0xAA]]);
            assert_eq!(spread.len(), 1);
            // Hop to ch149 → inject, todo now empty → entry dropped.
            assert_eq!(spread.drain_for_channel(149, 1_000_000), vec![vec![0xAA]]);
            assert!(spread.is_empty());

            // Subsequent hop to any channel yields nothing (already spread).
            assert!(spread.drain_for_channel(6, 1_000_000).is_empty());
            assert!(spread.drain_for_channel(44, 1_000_000).is_empty());
        }

        #[test]
        fn spread_dedupes_duplicate_channels_in_sequence() {
            // If the master chanseq lists the same channel on multiple slots,
            // the announce is sent once per DISTINCT channel, not once per
            // slot.
            let mut spread = MulticastSpread::new(MULTICAST_SPREAD_CAP, HOP_CYCLE_US);
            // Sequence full of 6s with a couple of 44s.
            let mut seq = [[6u8, 0u8]; CHANSEQ_LEN];
            seq[5] = [44, 0];
            seq[11] = [44, 0];
            assert!(spread.enqueue(vec![0xBB], &seq, 44, 1_000_000));

            // First time we hit ch6 → inject. Second time → nothing.
            assert_eq!(spread.drain_for_channel(6, 1_000_000), vec![vec![0xBB]]);
            assert!(spread.drain_for_channel(6, 1_000_000).is_empty());
            // ch44 → inject once, then done (todo empty).
            assert_eq!(spread.drain_for_channel(44, 1_000_000), vec![vec![0xBB]]);
            assert!(spread.is_empty());
        }

        #[test]
        fn spread_falls_back_to_single_anchor_channel() {
            // No master adopted: distinct_channels = [anchor], so the frame
            // is sent once on the anchor and then dropped — same observable
            // behaviour as the pre-spread code.
            let mut spread = MulticastSpread::new(MULTICAST_SPREAD_CAP, HOP_CYCLE_US);
            let seq = [[0u8, 0u8]; CHANSEQ_LEN]; // all null
            assert!(spread.enqueue(vec![0xCC], &seq, 44, 1_000_000));
            assert_eq!(spread.drain_for_channel(44, 1_000_000), vec![vec![0xCC]]);
            assert!(spread.is_empty());
        }

        #[test]
        fn spread_evicts_oldest_when_full() {
            // A bounded queue: when full, the oldest entry is dropped so the
            // newest announce still gets its hop cycle (no unbounded growth).
            let mut spread = MulticastSpread::new(2, HOP_CYCLE_US);
            let seq = chanseq(&[(6, 0), (44, 0)]);
            assert!(spread.enqueue(vec![0x01], &seq, 44, 1_000_000));
            assert!(spread.enqueue(vec![0x02], &seq, 44, 1_000_001));
            assert_eq!(spread.len(), 2);
            // Third enqueue evicts the first (0x01).
            assert!(spread.enqueue(vec![0x03], &seq, 44, 1_000_002));
            assert_eq!(spread.len(), 2);
            // Draining ch6 should yield only 0x02 and 0x03 (0x01 was evicted).
            let drained = spread.drain_for_channel(6, 1_000_002);
            assert_eq!(drained, vec![vec![0x02], vec![0x03]]);
        }

        #[test]
        fn spread_ages_out_stale_entries_past_one_cycle() {
            // If a frame's remaining channels never recur (e.g. the master
            // sequence changed mid-cycle), the TTL guard drops it instead of
            // leaking forever.
            let mut spread = MulticastSpread::new(MULTICAST_SPREAD_CAP, HOP_CYCLE_US);
            let seq = chanseq(&[(6, 0), (44, 0), (149, 0)]);
            assert!(spread.enqueue(vec![0xDD], &seq, 44, 1_000_000));
            // Channel 149 never recurs; advance past one hop cycle.
            let stale = 1_000_000 + HOP_CYCLE_US + 1;
            // Drain on a channel that IS in the todo: aged out → not returned.
            assert!(spread.drain_for_channel(6, stale).is_empty());
            assert!(spread.is_empty());
        }

        #[test]
        fn spread_enqueue_returns_false_when_no_target_channels() {
            // Defensive: all-zero sequence AND zero anchor → nothing to send.
            let mut spread = MulticastSpread::new(MULTICAST_SPREAD_CAP, HOP_CYCLE_US);
            let seq = [[0u8, 0u8]; CHANSEQ_LEN];
            assert!(!spread.enqueue(vec![0xEE], &seq, 0, 1_000_000));
            assert!(spread.is_empty());
        }

        // --- Bug C: transfer pin ---

        #[test]
        fn transfer_active_within_idle_timeout() {
            // Recent unicast RX from peer P on ch44 → pin to (P, 44).
            let pin = TransferPin {
                peer: [0x02, 1, 2, 3, 4, 5],
                channel: 44,
                last_rx_us: 1_000_000,
            };
            assert_eq!(
                transfer_active(Some(&pin), 1_000_000, TRANSFER_IDLE_TIMEOUT_US),
                Some(([0x02, 1, 2, 3, 4, 5], 44))
            );
        }

        #[test]
        fn transfer_releases_after_idle_timeout() {
            // Once the idle timeout elapses with no further RX, the pin
            // releases so filin can resume hopping and re-adopting.
            let pin = TransferPin {
                peer: [0x02, 1, 2, 3, 4, 5],
                channel: 44,
                last_rx_us: 1_000_000,
            };
            let after = 1_000_000 + TRANSFER_IDLE_TIMEOUT_US + 1;
            assert_eq!(
                transfer_active(Some(&pin), after, TRANSFER_IDLE_TIMEOUT_US),
                None
            );
        }

        #[test]
        fn transfer_inactive_when_no_pin_or_null_channel() {
            // No pin → not active.
            assert_eq!(transfer_active(None, 1_000_000, TRANSFER_IDLE_TIMEOUT_US), None);
            // A pin on channel 0 (null) → not active.
            let pin = TransferPin {
                peer: [0x02, 1, 2, 3, 4, 5],
                channel: 0,
                last_rx_us: 1_000_000,
            };
            assert_eq!(transfer_active(Some(&pin), 1_000_000, TRANSFER_IDLE_TIMEOUT_US), None);
        }

        // --- Bug B: master-adoption hysteresis ---

        const COMMITTED: [u8; 6] = [0x02, 0xaa, 0, 0, 0, 1];
        const CHALLENGER: [u8; 6] = [0x02, 0xbb, 0, 0, 0, 2];

        #[test]
        fn hysteresis_single_challenger_frame_does_not_flip_master() {
            // A single challenger win must NOT flip the committed master.
            let since = 1_000_000;
            let now = since + 1; // far less than the debounce window
            assert_eq!(
                master_adoption_decision(
                    COMMITTED,
                    CHALLENGER,
                    CHALLENGER,
                    since,
                    now,
                    MASTER_DEBOUNCE_US
                ),
                MasterAdoption::Keep
            );
        }

        #[test]
        fn hysteresis_sustained_challenger_wins_flip_master() {
            // A challenger that has won for the whole debounce window does
            // flip the committed master.
            let since = 1_000_000;
            let now = since + MASTER_DEBOUNCE_US;
            assert_eq!(
                master_adoption_decision(
                    COMMITTED,
                    CHALLENGER,
                    CHALLENGER,
                    since,
                    now,
                    MASTER_DEBOUNCE_US
                ),
                MasterAdoption::Adopt
            );
        }

        #[test]
        fn hysteresis_re_adopting_committed_master_is_noop_adopt() {
            // The winner being the already-committed master is an immediate
            // Adopt (refresh, clears the pending challenger).
            assert_eq!(
                master_adoption_decision(
                    COMMITTED,
                    COMMITTED,
                    CHALLENGER,
                    1_000_000,
                    1_000_000,
                    MASTER_DEBOUNCE_US
                ),
                MasterAdoption::Adopt
            );
        }

        #[test]
        fn hysteresis_unknown_challenger_does_not_adopt_immediately() {
            // A brand-new winner that is neither committed nor the tracked
            // pending challenger must Keep (start its debounce clock, not
            // adopt on the first frame).
            assert_eq!(
                master_adoption_decision(
                    COMMITTED,
                    CHALLENGER,
                    [0x02, 0xcc, 0, 0, 0, 3], // a different pending
                    1_000_000,
                    1_000_000 + MASTER_DEBOUNCE_US,
                    MASTER_DEBOUNCE_US
                ),
                MasterAdoption::Keep
            );
        }

        // --- ensure_social_coverage: guarantee ch149 coverage ---

        #[test]
        fn social_coverage_injects_missing_149_into_null_slot() {
            // Core regression: a master chanseq of all 44s + some nulls.
            // 6 and 149 are missing. After ensure_social_coverage, all
            // three social channels appear — 149 is injected so the radio
            // actually visits it during the hop cycle.
            let mut seq = [[44u8, 0x80]; CHANSEQ_LEN];
            // Make some slots null so there's room to inject.
            seq[5] = [0, 0];
            seq[6] = [0, 0];
            let injected = ensure_social_coverage(&mut seq);
            assert!(injected.contains(&6));
            assert!(injected.contains(&149));
            // 149 and 6 now appear in the sequence.
            assert!(seq.iter().any(|s| s[0] == 149));
            assert!(seq.iter().any(|s| s[0] == 6));
            assert!(seq.iter().any(|s| s[0] == 44)); // 44 still there
            // No more null slots (we filled them).
            assert!(!seq.iter().any(|s| s[0] == 0));
        }

        #[test]
        fn social_coverage_overwrites_duplicate_when_no_nulls() {
            // A master chanseq full of 44s — no nulls, no 6, no 149.
            // ensure_social_coverage must overwrite DUPLICATE 44 slots to
            // inject 6 and 149 without removing 44 entirely.
            let mut seq = [[44u8, 0x80]; CHANSEQ_LEN];
            ensure_social_coverage(&mut seq);
            assert!(seq.iter().any(|s| s[0] == 149));
            assert!(seq.iter().any(|s| s[0] == 6));
            assert!(seq.iter().any(|s| s[0] == 44), "44 must survive");
        }

        #[test]
        fn social_coverage_noop_when_all_present() {
            // A sequence that already has 6, 44, and 149 → nothing injected.
            let mut seq = [[0u8, 0u8]; CHANSEQ_LEN];
            seq[0] = [6, 0x51];
            seq[1] = [44, 0x80];
            seq[2] = [149, 0x80];
            let injected = ensure_social_coverage(&mut seq);
            assert!(injected.is_empty());
        }

        #[test]
        fn social_coverage_preserves_distinct_non_social_channels() {
            // If the master has a non-social channel (e.g. 36), the inject
            // must not remove it — it should use null slots, not overwrite
            // the only occurrence of a distinct channel.
            let mut seq = [[0u8, 0u8]; CHANSEQ_LEN];
            seq[0] = [36, 0x80]; // non-social, single occurrence
            // 5 null slots for 6, 44, 149.
            let injected = ensure_social_coverage(&mut seq);
            assert_eq!(injected.len(), 3);
            assert!(seq.iter().any(|s| s[0] == 36), "ch36 must survive");
        }

        // --- Part B: denser social-channel dwell (fill nulls round-robin) ---

        #[test]
        fn social_coverage_fills_nulls_round_robin_for_dense_149() {
            // A master seq with many nulls: after ensure_social_coverage, 149
            // appears in MORE than one slot (round-robin fill starting at
            // 149), so filin dwells on 149 several times per cycle.
            let mut seq = [[0u8, 0u8]; CHANSEQ_LEN];
            seq[0] = [44, 0x80]; // one real master channel
            // 15 nulls. ensure_social_coverage injects the missing 6/149
            // (44 already present), then fills the remaining 13 nulls
            // round-robin 149/6/44 starting at 149.
            ensure_social_coverage(&mut seq);
            let count_149 = seq.iter().filter(|s| s[0] == 149).count();
            assert!(
                count_149 > 1,
                "149 must appear in >1 slot (got {})",
                count_149
            );
            // No null slots remain.
            assert!(!seq.iter().any(|s| s[0] == 0), "no nulls remain");
            // 44 (master's real channel) survives.
            assert!(seq.iter().any(|s| s[0] == 44));
        }

        #[test]
        fn social_coverage_round_robin_starts_at_149() {
            // The round-robin starts at 149 so 149 gets the most slots.
            // With exactly 3 nulls and all three socials missing, each gets
            // exactly one — but 149 is first.
            let mut seq = [[44u8, 0x80]; CHANSEQ_LEN];
            seq[0] = [0, 0];
            seq[1] = [0, 0];
            seq[2] = [0, 0];
            // 44 already present (13 slots); 6 and 149 missing → 2 injected
            // into nulls; the 3rd null filled round-robin (starts at 149).
            ensure_social_coverage(&mut seq);
            // No nulls remain.
            assert!(!seq.iter().any(|s| s[0] == 0));
            // 149 present at least twice (1 injected + 1 round-robin).
            let count_149 = seq.iter().filter(|s| s[0] == 149).count();
            assert!(count_149 >= 2, "149 should get round-robin dwell");
        }

        #[test]
        fn social_coverage_does_not_touch_non_null_master_slots() {
            // Master-advertised channels in non-null slots must survive the
            // round-robin fill — only nulls are overwritten.
            let mut seq = [[0u8, 0u8]; CHANSEQ_LEN];
            seq[0] = [36, 0x80]; // non-social master channel
            seq[5] = [149, 0x80]; // social, already present
            ensure_social_coverage(&mut seq);
            assert_eq!(seq[0], [36, 0x80], "master ch36 slot untouched");
            assert_eq!(seq[5], [149, 0x80], "master ch149 slot untouched");
        }

        // --- Part A: AnnounceCache cached mDNS re-broadcast ---

        #[test]
        fn announce_cache_rebroadcast_on_social_visit() {
            // A TAP multicast frame is cached; visiting a social channel
            // re-broadcasts it.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA, 0xBB], 1_000_000);
            assert_eq!(cache.rebroadcast(149, 1_500_000), Some(&[0xAA, 0xBB][..]));
        }

        #[test]
        fn announce_cache_rate_limits_within_window() {
            // A second re-broadcast on the SAME channel within the window is
            // suppressed; the rate-limit is per-channel.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA], 1_000_000);
            assert!(cache.rebroadcast(149, 1_500_000).is_some());
            assert!(
                cache.rebroadcast(149, 1_900_000).is_none(),
                "second send within window must be suppressed"
            );
            // Past the window it fires again.
            assert!(cache.rebroadcast(149, 2_501_000).is_some());
        }

        #[test]
        fn announce_cache_rate_limit_is_per_channel() {
            // Re-broadcasting on ch6 must NOT rate-limit a subsequent ch149
            // send — the limit is per-channel so a dwell on 149 is never
            // blocked by a prior ch6 send.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA], 1_000_000);
            assert!(cache.rebroadcast(6, 1_500_000).is_some());
            assert!(
                cache.rebroadcast(149, 1_500_001).is_some(),
                "ch149 must not be rate-limited by a ch6 send"
            );
        }

        #[test]
        fn announce_cache_refresh_replaces_with_latest() {
            // A new TAP frame replaces the cached one.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA], 1_000_000);
            cache.refresh(vec![0xCC, 0xDD], 2_000_000);
            assert_eq!(cache.rebroadcast(149, 2_500_000), Some(&[0xCC, 0xDD][..]));
        }

        #[test]
        fn announce_cache_expires_when_stale() {
            // If no refresh arrives for longer than the TTL, the cache
            // stops re-broadcasting (dead receiver no longer advertised).
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA], 1_000_000);
            let stale = 1_000_000 + ANNOUNCE_CACHE_TTL_US + 1;
            assert!(cache.rebroadcast(149, stale).is_none());
            assert!(!cache.is_live(stale));
        }

        #[test]
        fn announce_cache_does_not_rebroadcast_on_non_social_channel() {
            // Only social channels (6/44/149) trigger a re-broadcast.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            cache.refresh(vec![0xAA], 1_000_000);
            assert!(cache.rebroadcast(36, 1_500_000).is_none());
        }

        #[test]
        fn announce_cache_empty_returns_none() {
            // With no frame cached, nothing is ever re-broadcast.
            let mut cache =
                AnnounceCache::new(ANNOUNCE_REBROADCAST_MIN_GAP_US, ANNOUNCE_CACHE_TTL_US);
            assert!(cache.rebroadcast(149, 1_000_000).is_none());
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
        /// The master whose channel sequence filin has committed to (Bug B
        /// hysteresis). Re-adoption only happens when a challenger wins the
        /// election for [`crate::schedule::MASTER_DEBOUNCE_US`].
        pub committed_master: [u8; 6],
        /// Challenger that won the most recent election but has not yet
        /// passed the debounce window (Bug B).
        pub pending_master: [u8; 6],
        /// When [`pending_master`] first won the election (Bug B).
        pub pending_since_us: u64,
        /// Sticky transfer pin: the peer/channel of the most recent unicast
        /// data RX (Bug C). While active, filin holds that channel and
        /// suppresses hopping + master re-adoption so a bulk AirDrop upload
        /// is not cut off mid-stream.
        pub transfer: Option<crate::schedule::TransferPin>,
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
                committed_master: self_addr,
                pending_master: self_addr,
                pending_since_us: now_us,
                transfer: None,
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
            // default static anchor sequence. Hysteresis (Bug B) and the
            // active-transfer pin (Bug C) gate re-adoption.
            self.sync_channel_sequence(rx_time_us);
        }

        /// If we have an elected master with a non-trivial channel sequence,
        /// adopt it as our own. When we lose the master, revert to the static
        /// anchor sequence. This ensures filin is on the same channel as the
        /// peer during each EAW slot.
        ///
        /// Bug B (hysteresis): a challenger must win the election for
        /// [`crate::schedule::MASTER_DEBOUNCE_US`] before filin commits to its
        /// channel sequence — stops the master thrash on a contended link.
        ///
        /// Bug C (transfer pin): while a unicast transfer is active, skip
        /// re-adoption entirely so an in-flight bulk upload is not
        /// descheduled.
        fn sync_channel_sequence(&mut self, now_us: u64) {
            // Bug C: an active unicast transfer freezes the committed master
            // (and thus the channel schedule) until the peer goes quiet.
            if crate::schedule::transfer_active(
                self.transfer.as_ref(),
                now_us,
                crate::schedule::TRANSFER_IDLE_TIMEOUT_US,
            )
            .is_some()
            {
                return;
            }
            let winner = self.election.sync_addr;
            // Bug B: debounce a challenger before committing — but only when
            // SWITCHING between two real external masters. The first adoption
            // (self → real master) and master loss (real master → self) are
            // immediate: neither is a flip-flop, so neither is debounced.
            let decision = if self.committed_master == self.self_addr
                || winner == self.self_addr
            {
                crate::schedule::MasterAdoption::Adopt
            } else {
                crate::schedule::master_adoption_decision(
                    self.committed_master,
                    winner,
                    self.pending_master,
                    self.pending_since_us,
                    now_us,
                    crate::schedule::MASTER_DEBOUNCE_US,
                )
            };
            match decision {
                crate::schedule::MasterAdoption::Adopt => {
                    self.committed_master = winner;
                    self.pending_master = winner;
                    self.pending_since_us = now_us;
                }
                crate::schedule::MasterAdoption::Keep => {
                    // Start/refresh the challenger's debounce clock.
                    if winner != self.pending_master {
                        self.pending_master = winner;
                        self.pending_since_us = now_us;
                    }
                    return; // hold the currently-committed master
                }
            }
            let master = self.committed_master;
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
                    let decoded: Vec<u8> =
                        peer.sequence.iter().map(|s| s[0]).collect();
                    self.channel_sequence = peer.sequence;
                    // Ensure filin visits ALL AWDL social channels (6, 44,
                    // 149) even if the master omits some — e.g. a master in
                    // an active data session may advertise only 44/6, but
                    // iPhones scan 149 for AirDrop discovery.
                    let injected = crate::schedule::ensure_social_coverage(
                        &mut self.channel_sequence,
                    );
                    let final_seq: Vec<u8> =
                        self.channel_sequence.iter().map(|s| s[0]).collect();
                    tracing::info!(
                        master = ?master,
                        decoded = ?decoded,
                        injected = ?injected,
                        final = ?final_seq,
                        "adopted master channel sequence"
                    );
                }
            }
        }

        /// owl `awdl_chan_num(state->channel.sequence[slot], enc)` — the channel
        /// number for the current EAW slot from our own sequence.
        ///
        /// Bug C: while a unicast transfer is active, returns the PINNED peer
        /// channel instead of the slot channel, so [`update_channel`] does not
        /// hop the radio away mid-upload.
        pub fn desired_channel(&self, now_us: u64) -> u8 {
            if let Some((_, channel)) = crate::schedule::transfer_active(
                self.transfer.as_ref(),
                now_us,
                crate::schedule::TRANSFER_IDLE_TIMEOUT_US,
            ) {
                return channel;
            }
            let slot = usize::from(self.sync.current_eaw(now_us) % CHANSEQ_LEN as u16);
            self.channel_sequence[slot][0]
        }

        /// Bug C: record a received unicast data frame from `peer` on
        /// `channel`, (re)arming the sticky transfer pin so filin holds this
        /// channel until the idle timeout elapses.
        pub fn note_unicast_rx(&mut self, peer: [u8; 6], channel: u8, now_us: u64) {
            self.transfer = Some(crate::schedule::TransferPin {
                peer,
                channel,
                last_rx_us: now_us,
            });
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
            self.sync_channel_sequence(now_us);
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

    /// Format a byte slice as a hex string for debug logging.
    fn hex_encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    /// owl `awdl_handle_chanseq_tlv` (owl/src/rx.c:60-109): store the peer's
    /// 16-slot channel sequence (as raw `(chan_num, opclass)` bytes).
    fn apply_chanseq(peer: &mut Peer, value: &[u8]) {
        if let Ok(seq) = awdl::parse_channel_sequence(value) {
            let decoded: Vec<u8> = seq.channels.iter().map(|&c| c as u8).collect();
            tracing::debug!(
                raw = %hex_encode(value),
                encoding = seq.encoding,
                decoded = ?decoded,
                peer = ?peer.addr,
                "chanseq TLV decoded"
            );
            for (slot, ch) in seq.channels.iter().take(CHANSEQ_LEN).enumerate() {
                peer.sequence[slot] = ch.to_le_bytes();
            }
        } else {
            tracing::debug!(
                raw = %hex_encode(value),
                peer = ?peer.addr,
                "chanseq TLV parse failed — peer sequence unchanged"
            );
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

        /// Like [`election_v2_tlv`] but with a configurable peer address, so
        /// two distinct masters can be made to compete in one state.
        fn election_v2_tlv_for(
            addr: [u8; 6],
            master_counter: u32,
            metric: u32,
        ) -> awdl::Tlv<'static> {
            let mut value = vec![0u8; 40];
            value[0..6].copy_from_slice(&addr); // master
            value[6..12].copy_from_slice(&addr); // sync (self)
            value[12..16].copy_from_slice(&master_counter.to_le_bytes());
            value[16..20].copy_from_slice(&0u32.to_le_bytes()); // distance
            value[20..24].copy_from_slice(&metric.to_le_bytes());
            value[24..28].copy_from_slice(&metric.to_le_bytes());
            value[36..40].copy_from_slice(&0u32.to_le_bytes());
            awdl::Tlv { kind: 24, value: value.leak() }
        }

        /// Build a chanseq TLV value (OPCLASS encoding) filled with `chan`.
        fn chanseq_value_all(chan: u8) -> Vec<u8> {
            let mut v = vec![15, awdl::CHAN_ENC_OPCLASS, 0, 3];
            v.extend_from_slice(&0xffffu16.to_le_bytes());
            for _ in 0..16 {
                v.extend_from_slice(&[chan, 0x80]);
            }
            v
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

            // filin should have adopted the master's sequence. The master
            // advertised 8x149 + 8x6; social coverage injected the missing
            // social channel 44. All three social channels are present.
            let chans: Vec<u8> =
                state.channel_sequence.iter().map(|s| s[0]).collect();
            assert!(chans.contains(&149), "149 from master");
            assert!(chans.contains(&6), "6 from master");
            assert!(chans.contains(&44), "44 injected by social coverage");
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
            // 149 present after adoption (social coverage injects 6/44 but
            // preserves at least one 149).
            assert!(state.channel_sequence.iter().any(|s| s[0] == 149));

            // master times out → revert to anchor (44)
            state.clean_peers(1_000 + peers::PEER_TIMEOUT_US + 1);
            assert_eq!(state.channel_sequence[0][0], 44);
        }

        #[test]
        fn adopts_legacy_chanseq_to_real_channels_not_flags_bytes() {
            // Regression for the live iPhone /Upload stall: a master that
            // advertises a LEGACY-encoded (encoding 1) channel sequence must
            // decode to REAL Wi-Fi channels, not the raw flags byte. filin
            // previously stored channel_sequence[slot][0] = flags (e.g.
            // 174/232/245), tuned the radio to invalid channels, and was
            // never co-channel with the iPhone for the sustained upload.
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_addr = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
            // LEGACY: [flags=0xa4, chan_num]. Channels 44 then 149.
            let mut chanseq_val = vec![15, awdl::CHAN_ENC_LEGACY, 0, 3];
            chanseq_val.extend_from_slice(&0xffffu16.to_le_bytes());
            for i in 0..16 {
                let ch = if i < 8 { 44u8 } else { 149u8 };
                chanseq_val.extend_from_slice(&[0xa4, ch]);
            }
            let tlvs = vec![
                election_v2_tlv(50, 200),
                version_tlv(0x34, 1),
                awdl::Tlv { kind: 18, value: &chanseq_val },
            ];
            state.apply_action(peer_addr, mif(), &tlvs, 1_000);
            // Real channels — NOT the 0xa4 flags byte (164) — and all three
            // social channels present after social-coverage injection.
            for slot in &state.channel_sequence {
                assert_ne!(slot[0], 0xa4, "no flags bytes in channel sequence");
            }
            let chans: Vec<u8> =
                state.channel_sequence.iter().map(|s| s[0]).collect();
            assert!(chans.contains(&44));
            assert!(chans.contains(&149));
            assert!(chans.contains(&6));
        }

        #[test]
        fn master_hysteresis_debounces_challenger_switch() {
            // Bug B: on a contended link two Apple devices compete; filin
            // must NOT re-adopt on every frame. A single challenger frame
            // holds; a sustained win over the debounce window commits.
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_a = [0x02, 0xaa, 0, 0, 0, 1];
            let peer_b = [0x02, 0xbb, 0, 0, 0, 2];

            // Adopt master A (counter 50, chan 149) — first adoption is
            // immediate (self → real master is not a flip-flop).
            let chan_a = chanseq_value_all(149);
            state.apply_action(
                peer_a,
                mif(),
                &[
                    election_v2_tlv_for(peer_a, 50, 200),
                    version_tlv(0x34, 1),
                    awdl::Tlv { kind: 18, value: &chan_a },
                ],
                1_000,
            );
            assert_eq!(state.committed_master, peer_a);
            // A's chanseq (16x149 → social coverage injects 6/44); 149
            // remains dominant (14 slots).
            assert!(
                state.channel_sequence.iter().filter(|s| s[0] == 149).count() >= 14,
                "A's chanseq must be in place"
            );

            // Master B (higher counter 60, chan 6) wins one frame — but it
            // is a challenger, so filin holds A.
            let chan_b = chanseq_value_all(6);
            state.apply_action(
                peer_b,
                mif(),
                &[
                    election_v2_tlv_for(peer_b, 60, 200),
                    version_tlv(0x34, 1),
                    awdl::Tlv { kind: 18, value: &chan_b },
                ],
                2_000,
            );
            assert_eq!(state.committed_master, peer_a, "single frame must not flip");
            assert!(
                state.channel_sequence.iter().filter(|s| s[0] == 149).count() >= 14,
                "chanseq must stay A's"
            );

            // B wins again after the debounce window → commit to B.
            let now = 2_000 + crate::schedule::MASTER_DEBOUNCE_US;
            state.apply_action(
                peer_b,
                mif(),
                &[
                    election_v2_tlv_for(peer_b, 60, 200),
                    version_tlv(0x34, 1),
                    awdl::Tlv { kind: 18, value: &chan_b },
                ],
                now,
            );
            assert_eq!(state.committed_master, peer_b, "sustained win must commit");
            // B's chanseq (16x6 → social coverage injects 44/149); 6
            // becomes dominant (14 slots).
            assert!(
                state.channel_sequence.iter().filter(|s| s[0] == 6).count() >= 14,
                "B's chanseq must be adopted"
            );
        }

        #[test]
        fn transfer_pin_holds_channel_and_suppresses_readoption() {
            // Bug C: while a unicast transfer is active, filin PINs the radio
            // to the peer's RX channel and suppresses hopping + master
            // re-adoption so the bulk upload is not cut off mid-stream.
            let mut state = AwdlState::new([0x02, 0, 0, 0, 0, 1], "filin".into(), 44, 0);
            let peer_a = [0x02, 0xaa, 0, 0, 0, 1];
            let peer_b = [0x02, 0xbb, 0, 0, 0, 2];

            // Adopt master A (chanseq all 149 → social coverage injects
            // 6/44 into early slots; slot 0 becomes 6, but the pin test
            // only cares that the pin overrides the slot channel).
            let chan_a = chanseq_value_all(149);
            state.apply_action(
                peer_a,
                mif(),
                &[
                    election_v2_tlv_for(peer_a, 50, 200),
                    version_tlv(0x34, 1),
                    awdl::Tlv { kind: 18, value: &chan_a },
                ],
                1_000,
            );
            // Slot 0 after social coverage is 6 (injected). desired_channel
            // returns the slot channel when no pin is active.
            let slot0_ch = state.desired_channel(1_000);
            assert!(slot0_ch != 0);

            // Arm the sticky pin: we just RX'd unicast from A on ch44.
            state.note_unicast_rx(peer_a, 44, 1_000);
            // desired_channel now returns the PINNED 44, not the slot
            // channel, so the runtime does not hop the radio away mid-upload.
            assert_eq!(state.desired_channel(1_000), 44);

            // Master B (higher counter, chan 6) tries to take over — the pin
            // suppresses re-adoption, so committed stays at A.
            let chan_b = chanseq_value_all(6);
            state.apply_action(
                peer_b,
                mif(),
                &[
                    election_v2_tlv_for(peer_b, 60, 200),
                    version_tlv(0x34, 1),
                    awdl::Tlv { kind: 18, value: &chan_b },
                ],
                1_100,
            );
            assert_eq!(state.committed_master, peer_a, "pin suppresses readoption");

            // After the idle timeout the pin releases; desired_channel
            // reverts to the slot channel and re-adoption can proceed.
            let after = 1_100 + crate::schedule::TRANSFER_IDLE_TIMEOUT_US + 1;
            assert_ne!(state.desired_channel(after), 44); // pin released
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
        monitor_iface: String,
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
            monitor_iface: config.monitor_iface.clone(),
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
        // owl keeps at most one pending unicast frame (`state->next`,
        // owl/daemon/core.c:42) and retries it when the AW/guard allows.
        let mut pending_tx: Option<PendingFrame> = None;
        // Multicast/broadcast frames (mDNS announce, NDP, broadcast) are
        // spread across every distinct channel of the master hop cycle so a
        // peer listening on any of those channels hears them. owl emits these
        // once on the current channel (`awdl_send_multicast`, core.c:246-277);
        // filin extends that to the full cycle because it channel-hops.
        let mut mcast_spread = crate::schedule::MulticastSpread::new(
            crate::schedule::MULTICAST_SPREAD_CAP,
            crate::schedule::HOP_CYCLE_US,
        );
        // Part A: cache of the latest multicast mDNS announce from the TAP,
        // re-broadcast on every social-channel visit so discovery does not
        // depend on luftlift's ~10s cadence aligning with a 149 hop.
        let mut announce_cache = crate::schedule::AnnounceCache::new(
            crate::schedule::ANNOUNCE_REBROADCAST_MIN_GAP_US,
            crate::schedule::ANNOUNCE_CACHE_TTL_US,
        );

        loop {
            let now_us = host_time_us();

            // --- channel switching (owl awdl_switch_channel, core.c:279) ---
            if announce.switch_due(&awdl_state.sync, now_us) {
                if let Some(channel) = awdl_state.update_channel(now_us) {
                    if let Some(freq) = channel::channel_to_frequency_mhz(channel) {
                        match os::nl80211::set_channel(&links.monitor_iface, u32::from(freq)) {
                            Ok(()) => tracing::debug!(
                                channel,
                                slot = awdl_state.sync.current_eaw(now_us) % 16,
                                "switched monitor channel"
                            ),
                            Err(err) => tracing::warn!(
                                ?err,
                                channel,
                                "failed to switch monitor channel"
                            ),
                        }
                    }
                }
            }

            // --- peer table cleanup (owl awdl_clean_peers, core.c:320) ---
            if announce.clean_due(now_us) {
                let removed = awdl_state.clean_peers(now_us);
                if !removed.is_empty() {
                    tracing::info!(
                        removed = removed.len(),
                        peers = awdl_state.peers.len(),
                        "peer table cleaned"
                    );
                }
            }

            // --- multicast spread TX: drain pending multicast/broadcast
            //     frames whose remaining channels include the current hop
            //     channel. Runs every iteration so a freshly-enqueued mDNS
            //     announce is injected on the current channel now AND on each
            //     distinct channel as filin hops through the master cycle
            //     (owl awdl_send_multicast, core.c:246-277, extended). ---
            if !mcast_spread.is_empty() {
                drain_mcast_spread(&links, &awdl_state, &mut mcast_spread, now_us);
            }

            // --- Part A: re-broadcast the cached mDNS announce on every
            //     social-channel visit (rate-limited ~1s/channel). Decouples
            //     discovery from luftlift's 10s cadence. Suppressed while a
            //     unicast transfer is pinned (Bug C). ---
            if announce_cache.is_live(now_us)
                && crate::schedule::transfer_active(
                    awdl_state.transfer.as_ref(),
                    now_us,
                    crate::schedule::TRANSFER_IDLE_TIMEOUT_US,
                )
                .is_none()
            {
                rebroadcast_announce(&links, &awdl_state, &mut announce_cache, now_us);
            }

            // --- action frame TX: PSF/MIF (owl awdl_send_psf/mif) ---
            for subtype in announce.due(now_us, &awdl_state) {
                let seq = awdl_state.next_data_seq();
                let frame = build_announce_frame(subtype, &awdl_state, seq << 4, now_us);
                match links.packet.send(&frame) {
                    Ok(sent) => tracing::trace!(sent, ?subtype, "injected awdl announce frame"),
                    Err(err) => tracing::trace!(?err, ?subtype, "packet announce send failed"),
                }
            }

            // --- data TX: retry pending frame if AW allows ---
            if pending_tx.is_some() {
                if let Some(frame) = pending_tx.take() {
                    if try_send_data(&links, &awdl_state, now_us, &frame) {
                        // sent successfully
                    } else {
                        // still can't send — keep it pending
                        pending_tx = Some(frame);
                    }
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
            // Only poll the tap if we don't already have a pending frame.
            if pending_tx.is_some() {
                fds[1].events = 0;
            }
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
                                    channel = awdl_state.current_channel,
                                    "rx awdl action frame"
                                );
                            }
                            Err(err) => match decapsulate_rx_data(&wlan_buf[..len]) {
                                Ok((source, frames)) => {
                                    if awdl_state.peers.get(&source).is_none() {
                                        tracing::trace!(
                                            ?source,
                                            "drop data frame from non-peer"
                                        );
                                    } else {
                                        for frame in &frames {
                                            links
                                                .tap
                                                .write_all(&frame.bytes)
                                                .map_err(|_| Error::Io)?;
                                        }
                                        // Bug C: arm the sticky transfer pin so
                                        // filin holds this peer's channel and
                                        // suppresses hopping/re-adoption for
                                        // the duration of the bulk upload.
                                        awdl_state.note_unicast_rx(
                                            source,
                                            awdl_state.current_channel,
                                            now_us,
                                        );
                                        tracing::trace!(
                                            count = frames.len(),
                                            channel = awdl_state.current_channel,
                                            "wrote ethernet frame(s) to tap"
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
                            let frame = PendingFrame {
                                radiotap_frame: radiotap_prefixed(ieee80211::build_data_frame(
                                    dst,
                                    src,
                                    AWDL_BSSID,
                                    seq << 4,
                                    &body,
                                )),
                                dst,
                            };
                            // Multicast/broadcast frames (mDNS announce,
                            // NDP, broadcast) are spread across every
                            // distinct channel of the master hop cycle so a
                            // peer listening on any channel hears them.
                            // Unicast keeps owl's AW-gated single-send path.
                            if frame.dst[0] & 0x01 == 1 {
                                // Part A: cache the latest multicast mDNS
                                // frame for periodic re-broadcast on social
                                // channels, decoupling discovery from
                                // luftlift's 10s cadence.
                                announce_cache
                                    .refresh(frame.radiotap_frame.clone(), host_time_us());
                                if mcast_spread.enqueue(
                                    frame.radiotap_frame,
                                    &awdl_state.channel_sequence,
                                    awdl_state.anchor_channel as u8,
                                    host_time_us(),
                                ) {
                                    tracing::trace!(
                                        pending = mcast_spread.len(),
                                        "enqueued multicast frame for hop-cycle spread"
                                    );
                                    // Inject on the current channel now too
                                    // (it is one of the target channels).
                                    drain_mcast_spread(
                                        &links,
                                        &awdl_state,
                                        &mut mcast_spread,
                                        host_time_us(),
                                    );
                                }
                            } else if !try_send_data(
                                &links,
                                &awdl_state,
                                host_time_us(),
                                &frame,
                            ) {
                                pending_tx = Some(frame);
                                tracing::trace!(
                                    pending = true,
                                    "buffered data frame for later AW-gated TX"
                                );
                            }
                        }
                        Err(err) => tracing::trace!(?err, len, "ignored host tap frame"),
                    }
                }
            }
        }
    }

    /// A frame waiting to be injected when the availability window allows.
    /// owl keeps one pending unicast frame in `state->next` (owl/daemon/core.c:42).
    struct PendingFrame {
        radiotap_frame: Vec<u8>,
        dst: [u8; 6],
    }

    /// owl `awdl_can_send_unicast_in` decision (owl/src/schedule.c:57-79):
    /// returns true if we are inside the guard-free core of an AW AND on the
    /// same channel as the destination peer (owl awdl_same_channel_as_peer).
    fn try_send_data(
        links: &Links,
        awdl_state: &crate::state::AwdlState,
        now_us: u64,
        frame: &PendingFrame,
    ) -> bool {
        let decision = crate::schedule::data_send_decision(
            &frame.dst,
            awdl_state.current_channel,
            &awdl_state.channel_sequence,
            &awdl_state.peers,
            &awdl_state.sync,
            now_us,
        );
        if decision != crate::schedule::DataSendDecision::Send {
            return false;
        }
        match links.packet.send(&frame.radiotap_frame) {
            Ok(sent) => {
                tracing::trace!(
                    sent,
                    channel = awdl_state.current_channel,
                    multicast = frame.dst[0] & 0x01 == 1,
                    "injected awdl data frame"
                );
                true
            }
            Err(err) => {
                tracing::trace!(?err, "data send failed");
                false
            }
        }
    }

    /// Inject any pending multicast-spread frames whose remaining channels
    /// include the current hop channel. Gated by the multicast guard interval
    /// (owl `awdl_can_send_in` with `AWDL_MULTICAST_GUARD_TU`) so we only
    /// inject in the sendable core of the current availability window; if we
    /// are inside the guard, the frames stay queued and are retried on the
    /// next iteration (or the next hop onto that channel). The pure
    /// scheduling decision lives in [`crate::schedule::MulticastSpread`].
    fn drain_mcast_spread(
        links: &Links,
        awdl_state: &crate::state::AwdlState,
        spread: &mut crate::schedule::MulticastSpread,
        now_us: u64,
    ) {
        let channel = awdl_state.current_channel;
        if channel == 0 {
            return; // null slot: nothing to transmit
        }
        if crate::schedule::can_send_in(
            &awdl_state.sync,
            now_us,
            crate::schedule::MULTICAST_GUARD_TU,
        ) != 0 {
            return; // inside multicast guard; retry next iteration
        }
        for frame in spread.drain_for_channel(channel, now_us) {
            match links.packet.send(&frame) {
                Ok(sent) => tracing::trace!(
                    sent,
                    channel,
                    pending = spread.len(),
                    "injected spread multicast frame on hop channel"
                ),
                Err(err) => tracing::trace!(?err, channel, "spread multicast send failed"),
            }
        }
    }

    /// Part A: re-inject the cached mDNS announce on the current channel if
    /// the [`AnnounceCache`] permits it (social channel, rate-limit ok, not
    /// expired). Gated by the multicast guard interval like the spread path.
    fn rebroadcast_announce(
        links: &Links,
        awdl_state: &crate::state::AwdlState,
        cache: &mut crate::schedule::AnnounceCache,
        now_us: u64,
    ) {
        let channel = awdl_state.current_channel;
        if channel == 0 {
            return;
        }
        if crate::schedule::can_send_in(
            &awdl_state.sync,
            now_us,
            crate::schedule::MULTICAST_GUARD_TU,
        ) != 0 {
            return; // inside multicast guard; retry next iteration
        }
        if let Some(frame) = cache.rebroadcast(channel, now_us) {
            match links.packet.send(frame) {
                Ok(sent) => tracing::debug!(
                    sent,
                    channel,
                    "re-broadcast cached mDNS announce on social channel"
                ),
                Err(err) => {
                    tracing::trace!(?err, channel, "announce re-broadcast send failed")
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
        next_switch_us: u64,
    }

    impl AnnounceScheduler {
        fn new(now_us: u64) -> Self {
            Self {
                next_psf_us: now_us + PSF_INTERVAL_TU * TU_US,
                // owl arms the MIF timer to fire immediately the first time
                // (core.c:419), then awdl_send_mif reschedules to mid-EAW.
                next_mif_us: now_us,
                next_clean_us: now_us + CLEAN_INTERVAL_US,
                // owl fires awdl_switch_channel immediately on start
                // (core.c:392), then rearms to next AW boundary.
                next_switch_us: now_us,
            }
        }

        fn poll_timeout_ms(&self, now_us: u64) -> i32 {
            let next = self
                .next_psf_us
                .min(self.next_mif_us)
                .min(self.next_clean_us)
                .min(self.next_switch_us);
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

        /// owl `awdl_switch_channel` timer (owl/daemon/core.c:279-308): fires at
        /// every AW boundary and rearms to `next_aw_us`.
        fn switch_due(&mut self, sync: &crate::sync::SyncState, now_us: u64) -> bool {
            if now_us >= self.next_switch_us {
                self.next_switch_us = now_us + sync.next_aw_us(now_us);
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

    /// Returns the source MAC and one or more decapsulated Ethernet frames.
    /// A-MSDU frames yield multiple Ethernet frames (owl awdl_rx_data_amsdu).
    fn decapsulate_rx_data(frame: &[u8]) -> Result<([u8; 6], Vec<awdl::EthernetFrame>), DataParseError> {
        let radiotap = radiotap::parse_header(frame).map_err(DataParseError::Radiotap)?;
        let ieee80211_payload =
            rx::ieee80211_payload_without_fcs(&radiotap).map_err(DataParseError::Fcs)?;
        let mac =
            ieee80211::parse_mac_header(ieee80211_payload).map_err(DataParseError::Ieee80211)?;
        // owl checks A-MSDU present bit in the QoS control field (rx.c:540).
        let is_amsdu = mac
            .qos_control
            .map(|q| q & ieee80211::QOS_A_MSDU_PRESENT != 0)
            .unwrap_or(false);
        let frames = if is_amsdu {
            awdl::decapsulate_amsdu_data_frames(mac.body).map_err(DataParseError::Awdl)?
        } else {
            vec![
                awdl::decapsulate_data_frame(mac.body, mac.source, mac.destination)
                    .map_err(DataParseError::Awdl)?,
            ]
        };
        Ok((mac.source, frames))
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
