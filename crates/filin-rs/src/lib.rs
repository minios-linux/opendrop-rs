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
