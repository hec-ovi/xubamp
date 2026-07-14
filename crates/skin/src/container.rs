//! `.wsz` skin container: a ZIP archive of loose files, looked up case-insensitively.
//!
//! Classic skins reference members by name (`main.bmp`, `cbuttons.bmp`, ...) with
//! unreliable casing and are otherwise flat. We parse the ZIP by hand (end-of-central-
//! directory record, central directory, local headers) and decompress members with
//! miniz_oxide, so the only dependency is a pure-Rust DEFLATE core. Sizes and offsets
//! come from the central directory, which is authoritative even when an entry uses a
//! data descriptor. Missing members return `None`; the renderer falls back to the
//! bundled default skin per missing file.

use std::collections::{HashMap, HashSet};

/// A loaded skin archive: lowercased basename -> raw file bytes, decompressed once.
pub struct SkinArchive {
    files: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// Required ZIP metadata is missing, inconsistent, or outside its declared bounds.
    BadArchive,
    /// The compressed archive itself exceeds the parser's input policy.
    InputTooLarge { size: usize, limit: usize },
    /// The central directory advertises more members than a skin may contain.
    TooManyEntries { entries: usize, limit: usize },
    /// Unique readable members would exceed the aggregate expanded-size policy
    /// according to their authoritative central-directory sizes.
    ExpandedTooLarge { size: usize, limit: usize },
}

// ZIP record signatures (little-endian u32).
const EOCD_SIG: u32 = 0x0605_4b50;
const CDFH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
/// Classic skins are normally a few megabytes with a few dozen members. These
/// deliberately generous ceilings preserve large custom skins while bounding every
/// attacker-controlled dimension before allocation or decompression.
const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_024;
const MAX_MEMBER_BYTES: usize = 32 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct ArchiveLimits {
    input_bytes: usize,
    entries: usize,
    member_bytes: usize,
    expanded_bytes: usize,
}

const DEFAULT_LIMITS: ArchiveLimits = ArchiveLimits {
    input_bytes: MAX_ARCHIVE_BYTES,
    entries: MAX_ENTRIES,
    member_bytes: MAX_MEMBER_BYTES,
    expanded_bytes: MAX_EXPANDED_BYTES,
};

impl SkinArchive {
    /// Load a `.wsz` (ZIP) from memory. Unreadable individual members are skipped;
    /// malformed container metadata or an archive-wide safety limit fails the load.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ArchiveError> {
        Self::from_bytes_with_limits(data, DEFAULT_LIMITS)
    }

    fn from_bytes_with_limits(data: &[u8], limits: ArchiveLimits) -> Result<Self, ArchiveError> {
        if data.len() > limits.input_bytes {
            return Err(ArchiveError::InputTooLarge {
                size: data.len(),
                limit: limits.input_bytes,
            });
        }

        let eocd = find_eocd(data).ok_or(ArchiveError::BadArchive)?;
        let disk = u16le_at(data, eocd, 4).ok_or(ArchiveError::BadArchive)?;
        let cd_disk = u16le_at(data, eocd, 6).ok_or(ArchiveError::BadArchive)?;
        let disk_total = u16le_at(data, eocd, 8).ok_or(ArchiveError::BadArchive)? as usize;
        let total = u16le_at(data, eocd, 10).ok_or(ArchiveError::BadArchive)? as usize;
        if disk != 0 || cd_disk != 0 || disk_total != total {
            return Err(ArchiveError::BadArchive); // multi-disk ZIP is unsupported
        }
        if total > limits.entries {
            return Err(ArchiveError::TooManyEntries {
                entries: total,
                limit: limits.entries,
            });
        }

        let cd_size = u32le_at(data, eocd, 12).ok_or(ArchiveError::BadArchive)? as usize;
        let cd_off = u32le_at(data, eocd, 16).ok_or(ArchiveError::BadArchive)? as usize;
        let cd_end = cd_off
            .checked_add(cd_size)
            .filter(|end| *end <= eocd)
            .ok_or(ArchiveError::BadArchive)?;

        let mut files = HashMap::with_capacity(total);
        let mut seen = HashSet::with_capacity(total);
        let mut expanded = 0usize;
        let mut pos = cd_off;
        for _ in 0..total {
            let fixed_end = pos
                .checked_add(46)
                .filter(|end| *end <= cd_end)
                .ok_or(ArchiveError::BadArchive)?;
            if u32le(data, pos) != Some(CDFH_SIG) {
                return Err(ArchiveError::BadArchive);
            }
            let flags = u16le_at(data, pos, 8).ok_or(ArchiveError::BadArchive)?;
            let method = u16le_at(data, pos, 10).ok_or(ArchiveError::BadArchive)?;
            let comp_size = u32le_at(data, pos, 20).ok_or(ArchiveError::BadArchive)? as usize;
            let uncomp_size = u32le_at(data, pos, 24).ok_or(ArchiveError::BadArchive)? as usize;
            let name_len = u16le_at(data, pos, 28).ok_or(ArchiveError::BadArchive)? as usize;
            let extra_len = u16le_at(data, pos, 30).ok_or(ArchiveError::BadArchive)? as usize;
            let comment_len = u16le_at(data, pos, 32).ok_or(ArchiveError::BadArchive)? as usize;
            let disk_start = u16le_at(data, pos, 34).ok_or(ArchiveError::BadArchive)?;
            let local_off = u32le_at(data, pos, 42).ok_or(ArchiveError::BadArchive)? as usize;

            let name_end = fixed_end
                .checked_add(name_len)
                .filter(|end| *end <= cd_end)
                .ok_or(ArchiveError::BadArchive)?;
            let next = name_end
                .checked_add(extra_len)
                .and_then(|end| end.checked_add(comment_len))
                .filter(|end| *end <= cd_end)
                .ok_or(ArchiveError::BadArchive)?;
            let name = data
                .get(fixed_end..name_end)
                .ok_or(ArchiveError::BadArchive)?;

            // Advance to the next central-directory entry before any `continue`.
            pos = next;

            if name.ends_with(b"/") || name.ends_with(b"\\") {
                continue; // directory entry
            }
            let base = basename_lower_bytes(name);
            if base.is_empty() {
                continue;
            }
            // ZIPs can contain the same flat skin member under many paths or cases.
            // First central-directory occurrence wins, and later duplicates must not
            // consume decompression work or aggregate output budget.
            if !seen.insert(base.clone()) {
                continue;
            }

            if disk_start != 0 || flags & 1 != 0 {
                continue; // split or encrypted member
            }
            if !matches!(method, METHOD_STORE | METHOD_DEFLATE)
                || comp_size > limits.member_bytes
                || uncomp_size > limits.member_bytes
                || (method == METHOD_STORE && comp_size != uncomp_size)
            {
                continue;
            }

            let Some(comp) = member_data(data, local_off, method, comp_size) else {
                continue;
            };
            let next_expanded =
                expanded
                    .checked_add(uncomp_size)
                    .ok_or(ArchiveError::ExpandedTooLarge {
                        size: usize::MAX,
                        limit: limits.expanded_bytes,
                    })?;
            if next_expanded > limits.expanded_bytes {
                return Err(ArchiveError::ExpandedTooLarge {
                    size: next_expanded,
                    limit: limits.expanded_bytes,
                });
            }

            if let Some(bytes) = decode_member(comp, method, uncomp_size) {
                expanded = next_expanded;
                files.insert(base, bytes);
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

/// Locate one member's compressed bytes after validating its local header. Sizes from
/// the central directory remain authoritative, including for data-descriptor entries.
fn member_data(data: &[u8], local_off: usize, method: u16, comp_size: usize) -> Option<&[u8]> {
    if u32le(data, local_off)? != LFH_SIG {
        return None;
    }
    if u16le_at(data, local_off, 8)? != method {
        return None;
    }
    // The local header's name/extra lengths can differ from the central directory's.
    let l_name = u16le_at(data, local_off, 26)? as usize;
    let l_extra = u16le_at(data, local_off, 28)? as usize;
    let start = local_off
        .checked_add(30)?
        .checked_add(l_name)?
        .checked_add(l_extra)?;
    data.get(start..start.checked_add(comp_size)?)
}

/// Decode a pre-bounded member and require the stream's actual output size to match
/// the authoritative central-directory size.
fn decode_member(comp: &[u8], method: u16, uncomp_size: usize) -> Option<Vec<u8>> {
    match method {
        METHOD_STORE => Some(comp.to_vec()),
        METHOD_DEFLATE => miniz_oxide::inflate::decompress_to_vec_with_limit(comp, uncomp_size)
            .ok()
            .filter(|bytes| bytes.len() == uncomp_size),
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
            let comment_len = u16le_at(data, i, 20)? as usize;
            if i.checked_add(22)
                .and_then(|end| end.checked_add(comment_len))
                == Some(data.len())
            {
                return Some(i);
            }
        }
        if i == earliest {
            return None;
        }
        i -= 1;
    }
}

fn basename_lower_bytes(name: &[u8]) -> String {
    let base = name
        .rsplit(|byte| matches!(byte, b'/' | b'\\'))
        .next()
        .unwrap_or_default();
    String::from_utf8_lossy(base).to_ascii_lowercase()
}

fn basename_lower(name: &str) -> String {
    basename_lower_bytes(name.as_bytes())
}

#[inline]
fn u16le(d: &[u8], o: usize) -> Option<u16> {
    let bytes = d.get(o..o.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}
#[inline]
fn u32le(d: &[u8], o: usize) -> Option<u32> {
    let bytes = d.get(o..o.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[inline]
fn u16le_at(d: &[u8], base: usize, rel: usize) -> Option<u16> {
    u16le(d, base.checked_add(rel)?)
}

#[inline]
fn u32le_at(d: &[u8], base: usize, rel: usize) -> Option<u32> {
    u32le(d, base.checked_add(rel)?)
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

    fn limits(member_bytes: usize, expanded_bytes: usize) -> ArchiveLimits {
        ArchiveLimits {
            input_bytes: usize::MAX,
            entries: usize::MAX,
            member_bytes,
            expanded_bytes,
        }
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

    #[test]
    fn rejects_input_over_archive_cap_before_parsing() {
        let wsz = build_zip(&[("main.bmp", b"main", METHOD_STORE)]);
        let archive_limits = ArchiveLimits {
            input_bytes: wsz.len() - 1,
            ..limits(usize::MAX, usize::MAX)
        };

        assert_eq!(
            SkinArchive::from_bytes_with_limits(&wsz, archive_limits).err(),
            Some(ArchiveError::InputTooLarge {
                size: wsz.len(),
                limit: wsz.len() - 1,
            })
        );
    }

    #[test]
    fn skips_oversized_stored_member_without_copying_it() {
        let wsz = build_zip(&[
            ("oversized.bmp", b"123456789", METHOD_STORE),
            ("main.bmp", b"small", METHOD_STORE),
        ]);
        let skin = SkinArchive::from_bytes_with_limits(&wsz, limits(8, 16)).unwrap();

        assert_eq!(skin.get("oversized.bmp"), None);
        assert_eq!(skin.get("main.bmp"), Some(&b"small"[..]));
    }

    #[test]
    fn rejects_unique_members_over_aggregate_expansion_cap() {
        let wsz = build_zip(&[
            ("main.bmp", b"12345678", METHOD_STORE),
            ("cbuttons.bmp", b"abcdefgh", METHOD_STORE),
        ]);

        assert_eq!(
            SkinArchive::from_bytes_with_limits(&wsz, limits(8, 12)).err(),
            Some(ArchiveError::ExpandedTooLarge {
                size: 16,
                limit: 12,
            })
        );
    }

    #[test]
    fn rejects_entry_count_before_allocating_member_maps() {
        let wsz = build_zip(&[
            ("main.bmp", b"main", METHOD_STORE),
            ("cbuttons.bmp", b"buttons", METHOD_STORE),
        ]);
        let archive_limits = ArchiveLimits {
            entries: 1,
            ..limits(usize::MAX, usize::MAX)
        };

        assert_eq!(
            SkinArchive::from_bytes_with_limits(&wsz, archive_limits).err(),
            Some(ArchiveError::TooManyEntries {
                entries: 2,
                limit: 1,
            })
        );
    }

    #[test]
    fn duplicate_basename_does_not_consume_expansion_budget() {
        let wsz = build_zip(&[
            ("MAIN.BMP", b"first---", METHOD_STORE),
            ("nested/main.bmp", b"second--", METHOD_STORE),
            ("CBUTTONS.BMP", b"buttons-", METHOD_STORE),
        ]);
        let skin = SkinArchive::from_bytes_with_limits(&wsz, limits(8, 16)).unwrap();

        assert_eq!(skin.len(), 2);
        assert_eq!(skin.get("main.bmp"), Some(&b"first---"[..]));
        assert_eq!(skin.get("cbuttons.bmp"), Some(&b"buttons-"[..]));
    }

    #[test]
    fn overflowing_central_metadata_is_rejected_without_panicking() {
        let mut wsz = build_zip(&[("main.bmp", b"main", METHOD_STORE)]);
        let eocd = find_eocd(&wsz).unwrap();
        let cd_off = u32le_at(&wsz, eocd, 16).unwrap() as usize;
        wsz[cd_off + 28..cd_off + 30].copy_from_slice(&u16::MAX.to_le_bytes());

        assert_eq!(
            SkinArchive::from_bytes_with_limits(&wsz, limits(usize::MAX, usize::MAX)).err(),
            Some(ArchiveError::BadArchive)
        );
        assert_eq!(u32le_at(&wsz, usize::MAX, usize::MAX), None);
    }
}
