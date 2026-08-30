//! Integrity check of the 512-byte factory calibration page.

/// Size of the factory calibration page (flash page 0).
pub const CALIBRATION_PAGE_LEN: usize = 512;

/// CRC-16/BUYPASS (a.k.a. CRC-16/UMTS): polynomial 0x8005, init 0x0000,
/// unreflected, no final XOR. The calibration page stores it big-endian in
/// its last two bytes.
pub fn crc16_buypass(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Whether a 512-byte calibration page carries a valid CRC: the last two
/// bytes (big-endian) must equal the CRC-16/BUYPASS of bytes `[0, 0x1FE)`.
/// The official application rejects a page failing this check; this driver
/// only warns, so a damaged page degrades to the nominal range model.
pub fn calibration_page_crc_ok(page: &[u8]) -> bool {
    if page.len() != CALIBRATION_PAGE_LEN {
        return false;
    }
    let stored = u16::from_be_bytes([
        page[CALIBRATION_PAGE_LEN - 2],
        page[CALIBRATION_PAGE_LEN - 1],
    ]);
    crc16_buypass(&page[..CALIBRATION_PAGE_LEN - 2]) == stored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_buypass_check_value() {
        // Standard check value for CRC-16/BUYPASS over "123456789".
        assert_eq!(crc16_buypass(b"123456789"), 0xFEE8);
    }

    #[test]
    fn a_page_with_its_own_crc_appended_validates_and_tampering_breaks_it() {
        let mut page = vec![0u8; CALIBRATION_PAGE_LEN];
        for (i, b) in page.iter_mut().enumerate().take(0x1FE) {
            *b = (i * 7 % 251) as u8;
        }
        let crc = crc16_buypass(&page[..0x1FE]);
        page[0x1FE..].copy_from_slice(&crc.to_be_bytes());
        assert!(calibration_page_crc_ok(&page));
        page[30] ^= 1;
        assert!(!calibration_page_crc_ok(&page));
        assert!(
            !calibration_page_crc_ok(&page[..100]),
            "wrong length is never valid"
        );
    }

    /// The real factory page served by the simulator must validate — the
    /// proof that the reverse-engineered CRC matches what hardware writes.
    #[cfg(feature = "sim")]
    #[test]
    fn the_simulators_real_factory_page_validates() {
        assert!(calibration_page_crc_ok(
            vqa40x_core::calpage::REAL_QA402_PAGE
        ));
    }
}
