//! Small, allocation-free ACPI table parser used by the x86 port.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub const MAX_CPUS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub apic_id: u32,
    pub enabled: bool,
    pub online_capable: bool,
    pub x2apic: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Madt {
    pub lapic_base: u64,
    pub bsp: u32,
    pub cpu_count: usize,
    pub cpus: [Cpu; MAX_CPUS],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    TooShort,
    BadSignature,
    BadChecksum,
    BadLength,
    MissingMadt,
    TooManyCpus,
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn check_sum(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn table<'a>(bytes: &'a [u8], signature: &[u8; 4]) -> Result<&'a [u8], Error> {
    if bytes.len() < 36 {
        return Err(Error::TooShort);
    }
    if &bytes[..4] != signature {
        return Err(Error::BadSignature);
    }
    let length = u32_at(bytes, 4) as usize;
    if length < 36 || length > bytes.len() {
        return Err(Error::BadLength);
    }
    let table = &bytes[..length];
    if !check_sum(table) {
        return Err(Error::BadChecksum);
    }
    Ok(table)
}

/// Parse RSDP v2, XSDT and MADT.  The caller supplies the three table images;
/// this keeps the parser usable before the kernel has a general heap/read map.
pub fn parse(rsdp: &[u8], xsdt: &[u8], madt: &[u8], madt_address: u64) -> Result<Madt, Error> {
    if rsdp.len() < 20 {
        return Err(Error::TooShort);
    }
    if &rsdp[..8] != b"RSD PTR " || rsdp[15] < 2 {
        return Err(Error::BadSignature);
    }
    let first = &rsdp[..20];
    if !check_sum(first) {
        return Err(Error::BadChecksum);
    }
    let revision = rsdp[15];
    let length = u32_at(rsdp, 20) as usize;
    if revision >= 2 {
        if length < 36 || length > rsdp.len() {
            return Err(Error::BadLength);
        }
        if !check_sum(&rsdp[..length]) {
            return Err(Error::BadChecksum);
        }
    }
    let xsdt = table(xsdt, b"XSDT")?;
    let mut found_madt = false;
    let mut offset = 36;
    while offset + 8 <= xsdt.len() {
        if u64_at(xsdt, offset) == madt_address {
            found_madt = true;
            break;
        }
        offset += 8;
    }
    if !found_madt {
        return Err(Error::MissingMadt);
    }
    let madt = table(madt, b"APIC")?;
    let mut result = Madt {
        lapic_base: u32_at(madt, 36) as u64,
        bsp: 0,
        cpu_count: 0,
        cpus: [Cpu {
            apic_id: 0,
            enabled: false,
            online_capable: false,
            x2apic: false,
        }; MAX_CPUS],
    };
    let mut offset = 44;
    while offset + 2 <= madt.len() {
        let entry_type = madt[offset];
        let entry_len = madt[offset + 1] as usize;
        if entry_len == 0 || offset + entry_len > madt.len() {
            return Err(Error::BadLength);
        }
        match entry_type {
            0 if entry_len >= 8 => {
                if result.cpu_count == MAX_CPUS {
                    return Err(Error::TooManyCpus);
                }
                let flags = u32_at(madt, offset + 4);
                let cpu = Cpu {
                    apic_id: madt[offset + 3] as u32,
                    enabled: flags & 1 != 0,
                    online_capable: flags & 2 != 0,
                    x2apic: false,
                };
                result.cpus[result.cpu_count] = cpu;
                result.cpu_count += 1;
            }
            9 if entry_len >= 16 => {
                if result.cpu_count == MAX_CPUS {
                    return Err(Error::TooManyCpus);
                }
                let flags = u32_at(madt, offset + 8);
                let cpu = Cpu {
                    apic_id: u32_at(madt, offset + 4),
                    enabled: flags & 1 != 0,
                    online_capable: flags & 2 != 0,
                    x2apic: true,
                };
                result.cpus[result.cpu_count] = cpu;
                result.cpu_count += 1;
            }
            5 if entry_len >= 12 => {
                result.lapic_base = u64_at(madt, offset + 4);
            }
            _ => {}
        }
        offset += entry_len;
    }
    for cpu in result.cpus[..result.cpu_count].iter() {
        if cpu.enabled {
            result.bsp = cpu.apic_id;
            break;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(bytes: &mut [u8]) {
        bytes[9] = 0;
        let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes[9] = 0u8.wrapping_sub(sum);
    }

    fn rsdp(xsdt_address: u64) -> [u8; 36] {
        let mut out = [0u8; 36];
        out[..8].copy_from_slice(b"RSD PTR ");
        out[15] = 2;
        out[16..20].copy_from_slice(&(0x1000u32).to_le_bytes());
        out[20..24].copy_from_slice(&(36u32).to_le_bytes());
        out[24..32].copy_from_slice(&xsdt_address.to_le_bytes());
        let sum = out[..20]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        out[8] = 0u8.wrapping_sub(sum);
        let sum = out.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        out[32] = 0u8.wrapping_sub(sum);
        out
    }

    fn xsdt(madt_address: u64) -> [u8; 44] {
        let mut out = [0u8; 44];
        out[..4].copy_from_slice(b"XSDT");
        out[4..8].copy_from_slice(&(44u32).to_le_bytes());
        out[8] = 1;
        out[36..44].copy_from_slice(&madt_address.to_le_bytes());
        checksum(&mut out);
        out
    }

    fn madt() -> std::vec::Vec<u8> {
        let mut out = std::vec![0u8; 44 + 8 + 8 + 12 + 16];
        out[..4].copy_from_slice(b"APIC");
        out[8] = 1;
        out[36..40].copy_from_slice(&0xfee0_0000u32.to_le_bytes());
        let mut offset = 44;
        // Type 0, two enabled CPUs.
        out[offset] = 0;
        out[offset + 1] = 8;
        out[offset + 2] = 0;
        out[offset + 3] = 0;
        out[offset + 4..offset + 8].copy_from_slice(&1u32.to_le_bytes());
        offset += 8;
        out[offset] = 0;
        out[offset + 1] = 8;
        out[offset + 2] = 1;
        out[offset + 3] = 1;
        out[offset + 4..offset + 8].copy_from_slice(&1u32.to_le_bytes());
        offset += 8;
        // Type 5 override.
        out[offset] = 5;
        out[offset + 1] = 12;
        out[offset + 4..offset + 12].copy_from_slice(&0xfee0_1000u64.to_le_bytes());
        offset += 12;
        // Type 9 x2APIC, disabled but online-capable.
        out[offset] = 9;
        out[offset + 1] = 16;
        out[offset + 4..offset + 8].copy_from_slice(&2u32.to_le_bytes());
        out[offset + 8..offset + 12].copy_from_slice(&2u32.to_le_bytes());
        let length = offset + 16;
        out[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        checksum(&mut out);
        out
    }

    fn parse_ok() -> (
        Madt,
        std::vec::Vec<u8>,
        std::vec::Vec<u8>,
        std::vec::Vec<u8>,
    ) {
        let xsdt_address = 0x2000u64;
        let madt_address = 0x3000u64;
        let r = rsdp(xsdt_address);
        let x = xsdt(madt_address);
        let m = madt();
        (
            parse(&r, &x, &m, 0x3000).unwrap(),
            r.to_vec(),
            x.to_vec(),
            m,
        )
    }

    #[test]
    fn valid_madt_has_two_enabled_and_one_x2apic() {
        let (parsed, _, _, _) = parse_ok();
        assert_eq!(parsed.lapic_base, 0xfee0_1000);
        assert_eq!(parsed.cpu_count, 3);
        assert!(parsed.cpus[0].enabled);
        assert!(parsed.cpus[1].enabled);
        assert_eq!(parsed.cpus[2].apic_id, 2);
        assert!(parsed.cpus[2].online_capable);
        assert!(!parsed.cpus[2].enabled);
    }

    #[test]
    fn bad_rsdp_checksum_is_rejected() {
        let (_, mut r, x, m) = parse_ok();
        r[8] ^= 1;
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap_err(), Error::BadChecksum);
    }

    #[test]
    fn bad_madt_checksum_is_rejected() {
        let (_, r, x, mut m) = parse_ok();
        m[9] ^= 1;
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap_err(), Error::BadChecksum);
    }

    #[test]
    fn zero_entry_length_is_rejected() {
        let (_, r, x, mut m) = parse_ok();
        let offset = 44;
        m[offset + 1] = 0;
        checksum(&mut m);
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap_err(), Error::BadLength);
    }

    #[test]
    fn entry_past_end_is_rejected() {
        let (_, r, x, mut m) = parse_ok();
        let offset = 44;
        m[offset + 1] = 19;
        checksum(&mut m);
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap_err(), Error::BadLength);
    }

    #[test]
    fn unknown_entry_is_skipped() {
        let (_, r, x, mut m) = parse_ok();
        m.extend_from_slice(&[0x7f, 2]);
        let length = m.len();
        m[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        checksum(&mut m);
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap().cpu_count, 3);
    }

    #[test]
    fn disabled_cpu_is_kept_but_not_bsp() {
        let (parsed, _, _, _) = parse_ok();
        assert_eq!(parsed.bsp, 0);
        assert!(!parsed.cpus[2].enabled);
    }

    #[test]
    fn missing_madt_in_xsdt_is_rejected() {
        let (_, r, _, m) = parse_ok();
        let x = xsdt(0x4000);
        assert_eq!(parse(&r, &x, &m, 0x3000).unwrap_err(), Error::MissingMadt);
    }
}
