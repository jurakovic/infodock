//! Reading skin packs from `.zip` files (SPEC §12.4): the central directory and entries that
//! are stored or deflated, enough for the archives Explorer and common tools write. Encrypted
//! entries and ZIP64 archives are refused; every entry is checked against its size and CRC.

use std::io::Read;

/// The central directory's end record and its signature.
const END_SIGNATURE: u32 = 0x0605_4B50;
const END_SIZE: usize = 22;
const CENTRAL_SIGNATURE: u32 = 0x0201_4B50;
const CENTRAL_SIZE: usize = 46;
const LOCAL_SIGNATURE: u32 = 0x0403_4B50;
const LOCAL_SIZE: usize = 30;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

/// Why an archive cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub enum ZipError {
    /// Not a zip file, or a damaged one.
    Invalid,
    /// ZIP64, encryption or a compression other than deflate.
    Unsupported,
}

/// A file in the archive.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The path in the archive, with `/` between folders; folders end with `/`.
    pub name: String,
    /// The size once extracted.
    pub size: u64,
    method: u16,
    encrypted: bool,
    crc: u32,
    compressed: u64,
    /// Where its local header starts.
    offset: u64,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The entries of the archive `bytes`, in the order of its central directory.
pub fn entries(bytes: &[u8]) -> Result<Vec<Entry>, ZipError> {
    // The end record is the last thing in the file, followed by a comment of up to 64 KiB.
    let earliest = bytes.len().saturating_sub(END_SIZE + usize::from(u16::MAX));
    let end = (earliest..=bytes.len().saturating_sub(END_SIZE))
        .rev()
        .find(|&at| u32_at(bytes, at) == Some(END_SIGNATURE))
        .ok_or(ZipError::Invalid)?;
    let count = u16_at(bytes, end + 10).ok_or(ZipError::Invalid)?;
    let start = u32_at(bytes, end + 16).ok_or(ZipError::Invalid)?;
    if count == u16::MAX || start == u32::MAX {
        return Err(ZipError::Unsupported);
    }
    let mut at = start as usize;
    let mut entries = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if u32_at(bytes, at) != Some(CENTRAL_SIGNATURE) {
            return Err(ZipError::Invalid);
        }
        let field16 = |offset| u16_at(bytes, at + offset).ok_or(ZipError::Invalid);
        let field32 = |offset| u32_at(bytes, at + offset).ok_or(ZipError::Invalid);
        let (flags, method) = (field16(8)?, field16(10)?);
        let (crc, compressed, size) = (field32(16)?, field32(20)?, field32(24)?);
        let name_len = usize::from(field16(28)?);
        let skip = name_len + usize::from(field16(30)?) + usize::from(field16(32)?);
        let offset = field32(42)?;
        if [compressed, size, offset].contains(&u32::MAX) {
            return Err(ZipError::Unsupported);
        }
        let name = bytes
            .get(at + CENTRAL_SIZE..at + CENTRAL_SIZE + name_len)
            .ok_or(ZipError::Invalid)?;
        entries.push(Entry {
            // Names are UTF-8 (bit 11) or code page 437; the latter matches for ASCII.
            name: String::from_utf8_lossy(name).replace('\\', "/"),
            size: u64::from(size),
            method,
            encrypted: flags & 1 != 0,
            crc,
            compressed: u64::from(compressed),
            offset: u64::from(offset),
        });
        at += CENTRAL_SIZE + skip;
    }
    Ok(entries)
}

/// Extracts `entry` from the archive `bytes`.
pub fn extract(bytes: &[u8], entry: &Entry) -> Result<Vec<u8>, ZipError> {
    if entry.encrypted || !matches!(entry.method, STORED | DEFLATED) {
        return Err(ZipError::Unsupported);
    }
    let at = usize::try_from(entry.offset).map_err(|_| ZipError::Invalid)?;
    if u32_at(bytes, at) != Some(LOCAL_SIGNATURE) {
        return Err(ZipError::Invalid);
    }
    let name_len = usize::from(u16_at(bytes, at + 26).ok_or(ZipError::Invalid)?);
    let extra_len = usize::from(u16_at(bytes, at + 28).ok_or(ZipError::Invalid)?);
    let start = at + LOCAL_SIZE + name_len + extra_len;
    let length = usize::try_from(entry.compressed).map_err(|_| ZipError::Invalid)?;
    let data = bytes
        .get(start..start.saturating_add(length))
        .ok_or(ZipError::Invalid)?;
    let size = usize::try_from(entry.size).map_err(|_| ZipError::Invalid)?;
    let out = if entry.method == STORED {
        data.to_vec()
    } else {
        // At most one byte more than declared, so a lying entry cannot inflate without end.
        let mut out = Vec::with_capacity(size);
        flate2::read::DeflateDecoder::new(data)
            .take(entry.size + 1)
            .read_to_end(&mut out)
            .map_err(|_| ZipError::Invalid)?;
        out
    };
    let mut crc = flate2::Crc::new();
    crc.update(&out);
    if out.len() != size || crc.sum() != entry.crc {
        return Err(ZipError::Invalid);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// A zip of `files` (name, contents), deflated when `deflate`, as tools write them.
    pub(crate) fn zip(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, contents) in files {
            let mut crc = flate2::Crc::new();
            crc.update(contents);
            let data = if deflate {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(contents).expect("deflated");
                encoder.finish().expect("deflated")
            } else {
                contents.to_vec()
            };
            let method: u16 = if deflate { DEFLATED } else { STORED };
            let offset = out.len() as u32;
            let header = |signature: u32, central: bool| {
                let mut h = signature.to_le_bytes().to_vec();
                if central {
                    h.extend(20u16.to_le_bytes()); // made by
                }
                h.extend(20u16.to_le_bytes()); // needed
                h.extend(0x0800u16.to_le_bytes()); // UTF-8 names
                h.extend(method.to_le_bytes());
                h.extend([0; 4]); // time, date
                h.extend(crc.sum().to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((contents.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes()); // extra
                if central {
                    h.extend([0; 6]); // comment, disk, internal attributes
                    h.extend([0; 4]); // external attributes
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(header(LOCAL_SIGNATURE, false));
            out.extend(&data);
            central.extend(header(CENTRAL_SIGNATURE, true));
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(END_SIGNATURE.to_le_bytes());
        out.extend([0; 4]); // disks
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes()); // comment
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let text = b"tile tile tile tile tile tile".as_slice();
        for deflate in [false, true] {
            let bytes = zip(&[("pack/", b""), ("pack/skin.json", text)], deflate);
            let entries = entries(&bytes).expect("a zip");
            let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["pack/", "pack/skin.json"]);
            assert_eq!(entries[1].size, text.len() as u64);
            assert_eq!(extract(&bytes, &entries[1]).as_deref(), Ok(text));
        }
    }

    #[test]
    fn refuses_damaged_and_unsupported_archives() {
        assert_eq!(entries(b"not a zip"), Err(ZipError::Invalid));
        assert_eq!(entries(b""), Err(ZipError::Invalid));
        let bytes = zip(&[("a.png", b"12345")], false);
        let entry = entries(&bytes).expect("a zip").remove(0);
        // A changed byte fails the CRC.
        let mut damaged = bytes.clone();
        damaged[LOCAL_SIZE + 5] ^= 1;
        assert_eq!(extract(&damaged, &entry), Err(ZipError::Invalid));
        // A size that does not match.
        let lying = Entry {
            size: 3,
            ..entry.clone()
        };
        assert_eq!(extract(&bytes, &lying), Err(ZipError::Invalid));
        let encrypted = Entry {
            encrypted: true,
            ..entry.clone()
        };
        assert_eq!(extract(&bytes, &encrypted), Err(ZipError::Unsupported));
        let bzip2 = Entry {
            method: 12,
            ..entry
        };
        assert_eq!(extract(&bytes, &bzip2), Err(ZipError::Unsupported));
    }
}
