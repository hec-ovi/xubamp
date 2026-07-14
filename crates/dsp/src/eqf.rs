//! Parser and creator for Winamp `.eqf` preset files and `Winamp.q1` libraries.

use std::fmt;

use crate::{EqSettings, MAX_DB, MIN_DB};

pub const HEADER: &[u8] = b"Winamp EQ library file v1.1";
const MARKER: &[u8] = b"\x1A!--";
const NAME_LEN: usize = 257;
const VALUE_COUNT: usize = 11;
const RECORD_LEN: usize = NAME_LEN + VALUE_COUNT;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    /// EQF semantic values, 1..=64: ten bands followed by preamp. On disk each byte is `64-value`.
    pub values: [u8; VALUE_COUNT],
}

impl Preset {
    pub fn from_settings(name: impl Into<String>, settings: EqSettings) -> Self {
        let settings = settings.sanitized();
        let mut values = [0; VALUE_COUNT];
        for (dst, db) in values[..10].iter_mut().zip(settings.bands_db) {
            *dst = db_to_value(db);
        }
        values[10] = db_to_value(settings.preamp_db);
        Self {
            name: name.into(),
            values,
        }
    }

    pub fn settings(&self, enabled: bool) -> EqSettings {
        let mut bands_db = [0.0; 10];
        for (dst, value) in bands_db.iter_mut().zip(self.values) {
            *dst = value_to_db(value);
        }
        EqSettings {
            enabled,
            preamp_db: value_to_db(self.values[10]),
            bands_db,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidHeader,
    InvalidMarker,
    TruncatedRecord {
        offset: usize,
        remaining: usize,
    },
    InvalidValue {
        preset: usize,
        value: usize,
        byte: u8,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidHeader => write!(f, "invalid Winamp EQF header"),
            Error::InvalidMarker => write!(f, "invalid Winamp EQF marker"),
            Error::TruncatedRecord { offset, remaining } => {
                write!(
                    f,
                    "truncated EQF preset at byte {offset} ({remaining} bytes remain)"
                )
            }
            Error::InvalidValue {
                preset,
                value,
                byte,
            } => {
                write!(
                    f,
                    "invalid EQF value byte {byte} in preset {preset}, slot {value}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

impl Library {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if !bytes.starts_with(HEADER) {
            return Err(Error::InvalidHeader);
        }
        let mut offset = HEADER.len();
        if bytes.get(offset..offset + MARKER.len()) != Some(MARKER) {
            return Err(Error::InvalidMarker);
        }
        offset += MARKER.len();
        let mut presets = Vec::new();
        while offset < bytes.len() {
            let remaining = bytes.len() - offset;
            if remaining < RECORD_LEN {
                return Err(Error::TruncatedRecord { offset, remaining });
            }
            let record = &bytes[offset..offset + RECORD_LEN];
            let end = record[..NAME_LEN]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(NAME_LEN);
            let name: String = record[..end].iter().map(|&b| char::from(b)).collect();
            let mut values = [0u8; VALUE_COUNT];
            for (i, (&byte, value)) in record[NAME_LEN..].iter().zip(&mut values).enumerate() {
                if byte > 63 {
                    return Err(Error::InvalidValue {
                        preset: presets.len(),
                        value: i,
                        byte,
                    });
                }
                *value = 64 - byte;
            }
            presets.push(Preset { name, values });
            offset += RECORD_LEN;
        }
        Ok(Self { presets })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes =
            Vec::with_capacity(HEADER.len() + MARKER.len() + self.presets.len() * RECORD_LEN);
        bytes.extend_from_slice(HEADER);
        bytes.extend_from_slice(MARKER);
        for preset in &self.presets {
            let mut name = Vec::with_capacity(NAME_LEN);
            for ch in preset.name.chars().take(NAME_LEN - 1) {
                name.push(u32::from(ch).try_into().unwrap_or(b'?'));
            }
            bytes.extend_from_slice(&name);
            bytes.resize(bytes.len() + NAME_LEN - name.len(), 0);
            for value in preset.values {
                bytes.push(64 - value.clamp(1, 64));
            }
        }
        bytes
    }
}

pub fn value_to_db(value: u8) -> f32 {
    let value = value.clamp(1, 64) as f32;
    MIN_DB + (value - 1.0) / 63.0 * (MAX_DB - MIN_DB)
}

pub fn db_to_value(db: f32) -> u8 {
    let db = if db.is_finite() {
        db.clamp(MIN_DB, MAX_DB)
    } else {
        0.0
    };
    (((db - MIN_DB) / (MAX_DB - MIN_DB) * 63.0).round() as u8 + 1).clamp(1, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        Library {
            presets: vec![
                Preset {
                    name: "Normal".into(),
                    values: [33; 11],
                },
                Preset {
                    name: "Min/Max".into(),
                    values: [1, 64, 2, 63, 3, 62, 4, 61, 5, 60, 32],
                },
            ],
        }
        .to_bytes()
    }

    #[test]
    fn parses_and_recreates_a_multi_preset_library_byte_for_byte() {
        let bytes = fixture();
        let parsed = Library::parse(&bytes).unwrap();
        assert_eq!(parsed.presets.len(), 2);
        assert_eq!(parsed.presets[0].name, "Normal");
        assert_eq!(parsed.presets[1].values[0], 1);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn rejects_bad_headers_markers_records_and_values() {
        assert_eq!(Library::parse(b"not an eqf"), Err(Error::InvalidHeader));
        let mut marker = fixture();
        marker[HEADER.len()] = 0;
        assert_eq!(Library::parse(&marker), Err(Error::InvalidMarker));
        let mut short = fixture();
        short.pop();
        assert!(matches!(
            Library::parse(&short),
            Err(Error::TruncatedRecord { .. })
        ));
        let mut value = fixture();
        value[HEADER.len() + MARKER.len() + NAME_LEN] = 64;
        assert!(matches!(
            Library::parse(&value),
            Err(Error::InvalidValue { .. })
        ));
    }

    #[test]
    fn settings_conversion_covers_the_full_classic_range() {
        assert_eq!(db_to_value(-12.0), 1);
        assert_eq!(db_to_value(12.0), 64);
        assert!((value_to_db(1) + 12.0).abs() < f32::EPSILON);
        assert!((value_to_db(64) - 12.0).abs() < f32::EPSILON);
        for value in 1..=64 {
            assert_eq!(db_to_value(value_to_db(value)), value);
        }
        let settings = EqSettings {
            enabled: false,
            preamp_db: 6.0,
            bands_db: [-12.0, -8.0, -4.0, 0.0, 4.0, 8.0, 12.0, 1.0, 2.0, 3.0],
        };
        let preset = Preset::from_settings("Entry1", settings);
        let restored = preset.settings(false);
        assert!((restored.preamp_db - settings.preamp_db).abs() < 0.2);
        for (actual, expected) in restored.bands_db.iter().zip(settings.bands_db) {
            assert!((actual - expected).abs() < 0.2);
        }
    }

    #[test]
    fn latin_one_names_round_trip_and_non_latin_is_replaced() {
        let library = Library {
            presets: vec![Preset {
                name: "Caf\u{00e9} \u{1f3b5}".into(),
                values: [33; 11],
            }],
        };
        let parsed = Library::parse(&library.to_bytes()).unwrap();
        assert_eq!(parsed.presets[0].name, "Caf\u{00e9} ?");
    }
}
