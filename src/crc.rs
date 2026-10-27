//! CRC-32 with zlib's polynomial, chained like `zlib.crc32(data, crc)`.

const POLY: u32 = 0xEDB8_8320;

const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Continues a CRC-32 from `crc` (0 to start) over `data`.
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_value() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn chaining_equals_one_pass() {
        let split = crc32(crc32(0, b"1234"), b"56789");
        assert_eq!(split, crc32(0, b"123456789"));
    }

    #[test]
    fn empty_keeps_the_crc() {
        assert_eq!(crc32(0, b""), 0);
        assert_eq!(crc32(0xDEAD_BEEF, b""), 0xDEAD_BEEF);
    }
}
