//! Strict ZIP32 framing: validate both copies of metadata and exact record
//! coverage. No extraction API, name normalization, or unbounded ZIP index.
use flate2::{Decompress, FlushDecompress, Status};

use super::{Error, Tree, portable_path};

struct Record<'a>(&'a [u8]);
impl<'a> Record<'a> {
    fn take(bytes: &'a [u8], offset: &mut usize, len: usize) -> Result<Self, Error> {
        let end = offset.checked_add(len).ok_or(Error::InvalidArchive)?;
        let data = bytes.get(*offset..end).ok_or(Error::InvalidArchive)?;
        *offset = end;
        Ok(Self(data))
    }
    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }
    fn u32(&self, at: usize) -> u32 {
        u32::from(self.u16(at)) | (u32::from(self.u16(at + 2)) << 16)
    }
    fn signature(&self, expected: u32) -> Result<(), Error> {
        if self.u32(0) != expected {
            return Err(Error::InvalidArchive);
        }
        Ok(())
    }
}

fn extras(bytes: &[u8], central: bool) -> Result<(), Error> {
    let mut offset = 0;
    let mut seen = 0;
    while offset < bytes.len() {
        let field = Record::take(bytes, &mut offset, 4)?;
        // Only ignorable timestamps and Unix UID/GID; notably not Unix link
        // metadata (0x000d/0x756e), ZIP64 (1), or encryption extensions.
        let bit = match field.u16(0) {
            0x5455 => 1,
            0x7875 => 2,
            0x000a => 4,
            _ => return Err(Error::UnsupportedEntry),
        };
        if seen & bit != 0 {
            return Err(Error::InvalidArchive);
        }
        seen |= bit;
        let data = Record::take(bytes, &mut offset, usize::from(field.u16(2)))?.0;
        let valid = match field.u16(0) {
            0x5455 => data.first().is_some_and(|flags| {
                flags & !7 == 0
                    && data.len()
                        == 1 + 4 * if central {
                            u32::from(flags & 1)
                        } else {
                            flags.count_ones()
                        } as usize
            }),
            0x7875 => {
                let mut at = 0;
                let version = Record::take(data, &mut at, 2)?;
                Record::take(data, &mut at, usize::from(version.0[1]))?;
                let gid = Record::take(data, &mut at, 1)?;
                Record::take(data, &mut at, usize::from(gid.0[0]))?;
                version.0[0] == 1 && version.0[1] != 0 && gid.0[0] != 0 && at == data.len()
            }
            // NTFS timestamps: reserved word, tag 1, three 64-bit times.
            0x000a => data.len() == 32 && data[..8] == [0, 0, 0, 0, 1, 0, 24, 0],
            _ => false,
        };
        if !valid {
            return Err(Error::InvalidArchive);
        }
    }
    Ok(())
}

pub(super) fn extract_zip(bytes: &[u8], tree: &mut Tree) -> Result<(), Error> {
    // EOCD is at most 65535 comment bytes from EOF. A candidate must cover EOF
    // exactly; never accept appended bytes, arbitrary prefixes, or split disks.
    let end = (bytes.len().saturating_sub(65557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            bytes.get(i..i + 4) == Some(b"PK\x05\x06")
                && i + 22 + usize::from(u16::from_le_bytes([bytes[i + 20], bytes[i + 21]]))
                    == bytes.len()
        })
        .ok_or(Error::InvalidArchive)?;
    let mut offset = end;
    let footer = Record::take(bytes, &mut offset, 22)?;
    let count = usize::from(footer.u16(10));
    if footer.u16(4) != 0 || footer.u16(6) != 0 || footer.u16(8) != footer.u16(10) {
        return Err(Error::UnsupportedEntry);
    }
    if count == 65535 || footer.u32(12) == u32::MAX || footer.u32(16) == u32::MAX {
        return Err(Error::UnsupportedEntry);
    }
    if count > tree.limits.entries {
        return Err(Error::LimitExceeded);
    }
    let start = footer.u32(16) as usize;
    if start.checked_add(footer.u32(12) as usize) != Some(end) {
        return Err(Error::InvalidArchive);
    }
    let central = bytes.get(start..end).ok_or(Error::InvalidArchive)?;
    let local = bytes.get(..start).ok_or(Error::InvalidArchive)?;
    let (mut index, mut position) = (0, 0);
    for _ in 0..count {
        let entry = Record::take(central, &mut index, 46)?;
        entry.signature(0x02014b50)?;
        let flags = entry.u16(8);
        let method = entry.u16(10);
        // Flags permit UTF-8, descriptors, and DEFLATE compression hints only.
        if flags & !0x080e != 0
            || !matches!(method, 0 | 8)
            || (method == 0 && flags & 6 != 0)
            || !matches!(entry.u16(6), 10 | 20)
            || ((method == 8 || flags & 8 != 0) && entry.u16(6) != 20)
            || entry.u16(34) != 0
            || entry.u16(36) & !1 != 0
        {
            return Err(Error::UnsupportedEntry);
        }
        let compressed = entry.u32(20) as usize;
        let size = entry.u32(24) as usize;
        if entry.u32(20) == u32::MAX || entry.u32(24) == u32::MAX {
            return Err(Error::UnsupportedEntry);
        }
        if size > tree.limits.bytes - tree.bytes {
            return Err(Error::LimitExceeded);
        }
        let name = Record::take(central, &mut index, usize::from(entry.u16(28)))?.0;
        let extra = Record::take(central, &mut index, usize::from(entry.u16(30)))?.0;
        Record::take(central, &mut index, usize::from(entry.u16(32)))?;
        let attributes = entry.u32(38);
        let host = entry.u16(4) >> 8;
        if !matches!(host, 0 | 3) || attributes & 0xffff & !0x37 != 0 {
            // DOS reparse/volume/device attributes and unknown hosts are unsafe.
            return Err(Error::UnsupportedEntry);
        }
        let mode = if host == 3 { attributes >> 16 } else { 0 };
        let kind = mode & 0o170000;
        if kind == 0o120000 {
            return Err(Error::UnsupportedLink);
        }
        if !matches!(kind, 0 | 0o100000 | 0o040000) || (host == 0 && attributes >> 16 != 0) {
            return Err(Error::UnsupportedEntry);
        }
        let directory = name.ends_with(b"/");
        if (kind == 0o040000 || attributes & 0x10 != 0) && !directory
            || (kind == 0o100000 && directory)
        {
            return Err(Error::InvalidArchive);
        }
        portable_path(name, directory, tree.limits)?;
        extras(extra, true)?;
        // Local records must be contiguous, unique, and in central-directory
        // order: this rejects overlaps, hidden unindexed entries and polyglots.
        if entry.u32(42) as usize != position {
            return Err(Error::InvalidArchive);
        }
        let header = Record::take(local, &mut position, 30)?;
        header.signature(0x04034b50)?;
        if header.u16(4) != entry.u16(6)
            || header.u16(6) != flags
            || header.u16(8) != method
            || header.u32(10) != entry.u32(12)
        {
            return Err(Error::InvalidArchive);
        }
        for (a, b) in [(14, 16), (18, 20), (22, 24)] {
            if header.u32(a) != entry.u32(b) && !(flags & 8 != 0 && header.u32(a) == 0) {
                return Err(Error::InvalidArchive);
            }
        }
        let local_name = Record::take(local, &mut position, usize::from(header.u16(26)))?.0;
        if local_name != name {
            return Err(Error::InvalidArchive);
        }
        extras(
            Record::take(local, &mut position, usize::from(header.u16(28)))?.0,
            false,
        )?;
        let payload = Record::take(local, &mut position, compressed)?.0;
        if flags & 8 != 0 {
            // Check unsigned first: the CRC itself may equal the signature.
            let mut descriptor = Record::take(local, &mut position, 12)?;
            let matches = |d: &Record<'_>| {
                d.u32(0) == entry.u32(16) && d.u32(4) == entry.u32(20) && d.u32(8) == entry.u32(24)
            };
            if !matches(&descriptor) && descriptor.u32(0) == 0x08074b50 {
                position -= 8;
                descriptor = Record::take(local, &mut position, 12)?;
            }
            if !matches(&descriptor) {
                return Err(Error::InvalidArchive);
            }
        }
        let mut decoded = Vec::new();
        let data = if method == 8 {
            // Fixed capacity includes one overflow byte; never trust a decoder
            // to allocate from malicious sizes or expand past the shared limit.
            decoded.resize(size + 1, 0);
            let mut decoder = Decompress::new(false);
            let status = decoder
                .decompress(payload, &mut decoded, FlushDecompress::Finish)
                .map_err(|_| Error::InvalidArchive)?;
            if status != Status::StreamEnd
                || decoder.total_in() != compressed as u64
                || decoder.total_out() != size as u64
            {
                return Err(Error::InvalidArchive);
            }
            &decoded[..size]
        } else {
            if compressed != size {
                return Err(Error::InvalidArchive);
            }
            payload
        };
        if crc32fast::hash(data) != entry.u32(16) {
            return Err(Error::InvalidArchive);
        }
        tree.entry(name, directory, mode, size as u64, data)?;
    }
    if index != central.len() || position != local.len() {
        return Err(Error::InvalidArchive);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
