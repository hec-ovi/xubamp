//! `.wsz` skin container: a ZIP archive of loose files, looked up case-insensitively.
//!
//! Classic skins reference members by name (`main.bmp`, `cbuttons.bmp`, ...) with
//! unreliable casing and are otherwise flat. We parse the ZIP by hand (end-of-central-
//! directory record, central directory, local headers) and decompress members with
//! miniz_oxide, so the only dependency is a pure-Rust DEFLATE core. Sizes and offsets
//! come from the central directory, which is authoritative even when an entry uses a
//! data descriptor. Missing members return `None`; the renderer falls back to the
//! bundled default skin per missing file.

use std::collections::HashMap;

/// A loaded skin archive: lowercased basename -> raw file bytes, decompressed once.
pub struct SkinArchive {
    files: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// No ZIP end-of-central-directory record was found.
    BadArchive,
}

// ZIP record signatures (little-endian u32).
const EOCD_SIG: u32 = 0x0605_4b50;
const CDFH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
/// Per-member decompressed-size cap. Classic skin sheets are tiny; this bounds a
/// corrupt or hostile uncompressed-size field.
const MAX_MEMBER_BYTES: usize = 32 * 1024 * 1024;

impl SkinArchive {
    /// Load a `.wsz` (ZIP) from memory. Unreadable individual members are skipped;
    /// only a missing end-of-central-directory record fails the whole load.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ArchiveError> {
        let eocd = find_eocd(data).ok_or(ArchiveError::BadArchive)?;
        let total = u16le(data, eocd + 10).unwrap_or(0) as usize;
        let cd_off = u32le(data, eocd + 16).ok_or(ArchiveError::BadArchive)? as usize;

        let mut files = HashMap::with_capacity(total);
        let mut pos = cd_off;
        for _ in 0..total {
            if u32le(data, pos) != Some(CDFH_SIG) {
                break;
            }
            let method = match u16le(data, pos + 10) {
                Some(v) => v,
                None => break,
            };
            let comp_size = match u32le(data, pos + 20) {
                Some(v) => v as usize,
                None => break,
            };
            let uncomp_size = u32le(data, pos + 24).unwrap_or(0) as usize;
            let name_len = u16le(data, pos + 28).unwrap_or(0) as usize;
            let extra_len = u16le(data, pos + 30).unwrap_or(0) as usize;
            let comment_len = u16le(data, pos + 32).unwrap_or(0) as usize;
            let local_off = match u32le(data, pos + 42) {
                Some(v) => v as usize,
                None => break,
            };
            let name = data
                .get(pos + 46..pos + 46 + name_len)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();

            // Advance to the next central-directory entry before any `continue`.
            pos = pos + 46 + name_len + extra_len + comment_len;

            if name.ends_with('/') {
                continue; // directory entry
            }
            let base = basename_lower(&name);
            if base.is_empty() {
                continue;
            }

            if let Some(bytes) = read_member(data, local_off, method, comp_size, uncomp_size) {
                files.entry(base).or_insert(bytes);
            }
        }

        Ok(Self { files })
    }

    /// Raw bytes of a member, matched case-insensitively on its basename.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(&basename_lower(name)).map(Vec::as_slice)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.files.contains_key(&basename_lower(name))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Read and decompress one member given its central-directory metadata. Returns
/// `None` if the local header is inconsistent or the data does not fit.
fn read_member(
    data: &[u8],
    local_off: usize,
    method: u16,
    comp_size: usize,
    uncomp_size: usize,
) -> Option<Vec<u8>> {
    if u32le(data, local_off)? != LFH_SIG {
        return None;
    }
    // The local header's name/extra lengths can differ from the central directory's.
    let l_name = u16le(data, local_off + 26)? as usize;
    let l_extra = u16le(data, local_off + 28)? as usize;
    let start = local_off + 30 + l_name + l_extra;
    let comp = data.get(start..start.checked_add(comp_size)?)?;

    match method {
        METHOD_STORE => Some(comp.to_vec()),
        METHOD_DEFLATE => {
            let limit = if uncomp_size == 0 {
                MAX_MEMBER_BYTES
            } else {
                uncomp_size.min(MAX_MEMBER_BYTES)
            };
            miniz_oxide::inflate::decompress_to_vec_with_limit(comp, limit).ok()
        }
        _ => None,
    }
}

/// Scan backward for the end-of-central-directory signature (the record sits at the
/// tail, after an optional comment of up to 0xffff bytes).
fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let earliest = data.len().saturating_sub(22 + 0xffff);
    let mut i = data.len() - 22;
    loop {
        if u32le(data, i) == Some(EOCD_SIG) {
            return Some(i);
        }
        if i == earliest {
            return None;
        }
        i -= 1;
    }
}

fn basename_lower(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[inline]
fn u16le(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*d.get(o)?, *d.get(o + 1)?]))
}
#[inline]
fn u32le(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *d.get(o)?,
        *d.get(o + 1)?,
        *d.get(o + 2)?,
        *d.get(o + 3)?,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble a minimal but valid ZIP from (name, original bytes, method). DEFLATE
    /// entries are compressed here with miniz_oxide, so the test round-trips our own
    /// reader against real ZIP structure.
    fn build_zip(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut count = 0u16;

        for (name, original, method) in entries {
            let comp: Vec<u8> = match *method {
                METHOD_DEFLATE => miniz_oxide::deflate::compress_to_vec(original, 6),
                _ => original.to_vec(),
            };
            let local_off = out.len() as u32;

            out.extend_from_slice(&LFH_SIG.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc (reader ignores)
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(original.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&comp);

            central.extend_from_slice(&CDFH_SIG.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // time
            central.extend_from_slice(&0u16.to_le_bytes()); // date
            central.extend_from_slice(&0u32.to_le_bytes()); // crc
            central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            central.extend_from_slice(&(original.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk start
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&local_off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
            count += 1;
        }

        let cd_off = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);

        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // this disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&count.to_le_bytes()); // entries this disk
        out.extend_from_slice(&count.to_le_bytes()); // total entries
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    #[test]
    fn reads_store_and_deflate_case_insensitively() {
        let big = vec![b'A'; 5000]; // compresses well, exercises the inflate path
        let wsz = build_zip(&[
            ("MAIN.BMP", b"main-bytes", METHOD_STORE),
            ("CButtons.bmp", &big, METHOD_DEFLATE),
        ]);
        let skin = SkinArchive::from_bytes(&wsz).unwrap();
        assert_eq!(skin.len(), 2);
        assert!(!skin.is_empty());
        assert_eq!(skin.get("main.bmp"), Some(&b"main-bytes"[..]));
        assert_eq!(skin.get("MAIN.BMP"), Some(&b"main-bytes"[..]));
        assert_eq!(skin.get("cbuttons.bmp"), Some(big.as_slice())); // deflate round-trip
        assert!(skin.contains("Main.Bmp"));
        assert_eq!(skin.get("missing.bmp"), None);
    }

    #[test]
    fn flattens_subfolder_paths() {
        let wsz = build_zip(&[("skin/Region.txt", b"pts", METHOD_STORE)]);
        let skin = SkinArchive::from_bytes(&wsz).unwrap();
        assert_eq!(skin.get("region.txt"), Some(&b"pts"[..]));
    }

    #[test]
    fn rejects_non_zip() {
        assert_eq!(
            SkinArchive::from_bytes(b"this is definitely not a zip file").err(),
            Some(ArchiveError::BadArchive)
        );
    }
}
