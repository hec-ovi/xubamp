//! Built-in classic preset data. Values use the EQF 1..=64 scale and are converted through the
//! same path as loaded files, so menu presets and `.eqf` presets cannot drift.

use crate::eqf::Preset;

const DATA: [(&str, [u8; 10]); 17] = [
    ("Classical", [33, 33, 33, 33, 33, 33, 20, 20, 20, 16]),
    ("Club", [33, 33, 38, 42, 42, 42, 38, 33, 33, 33]),
    ("Dance", [48, 44, 36, 32, 32, 22, 20, 20, 32, 32]),
    (
        "Laptop speakers/headphones",
        [40, 50, 41, 26, 28, 35, 40, 48, 53, 56],
    ),
    ("Large hall", [49, 49, 42, 42, 33, 24, 24, 24, 33, 33]),
    ("Party", [44, 44, 33, 33, 33, 33, 33, 33, 44, 44]),
    ("Pop", [29, 40, 44, 45, 41, 30, 28, 28, 29, 29]),
    ("Reggae", [33, 33, 31, 22, 33, 43, 43, 33, 33, 33]),
    ("Rock", [45, 40, 23, 19, 26, 39, 47, 50, 50, 50]),
    ("Soft", [40, 35, 30, 28, 30, 39, 46, 48, 50, 52]),
    ("Ska", [28, 24, 25, 31, 39, 42, 47, 48, 50, 48]),
    ("Full Bass", [48, 48, 48, 42, 35, 25, 18, 15, 14, 14]),
    ("Soft Rock", [39, 39, 36, 31, 25, 23, 26, 31, 37, 47]),
    ("Full Treble", [16, 16, 16, 25, 37, 50, 58, 58, 58, 60]),
    (
        "Full Bass & Treble",
        [44, 42, 33, 20, 24, 35, 46, 50, 52, 52],
    ),
    ("Live", [24, 33, 39, 41, 42, 42, 39, 37, 37, 36]),
    ("Techno", [45, 42, 33, 23, 24, 33, 45, 48, 48, 47]),
];

pub fn builtins() -> Vec<Preset> {
    DATA.iter()
        .map(|(name, bands)| {
            let mut values = [33; 11];
            values[..10].copy_from_slice(bands);
            Preset {
                name: (*name).into(),
                values,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_order_and_names_match_the_reference_surface() {
        let presets = builtins();
        assert_eq!(presets.len(), 17);
        assert_eq!(presets.first().unwrap().name, "Classical");
        assert_eq!(presets[3].name, "Laptop speakers/headphones");
        assert_eq!(presets[14].name, "Full Bass & Treble");
        assert_eq!(presets.last().unwrap().name, "Techno");
        assert!(presets.iter().all(|preset| preset.values[10] == 33));
        assert!(presets
            .iter()
            .flat_map(|preset| preset.values)
            .all(|value| (1..=64).contains(&value)));
    }
}
