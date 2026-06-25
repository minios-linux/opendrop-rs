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
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct Tlv<'a> {
        pub kind: u8,
        pub value: &'a [u8],
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

    pub fn parse_tlvs(_bytes: &[u8]) -> Result<Vec<Tlv<'_>>, ParseError> {
        Err(ParseError::TooShort)
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
    }
}
