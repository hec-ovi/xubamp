//! `region.txt`: the non-rectangular window outlines for shaped skins. Each section
//! (`[Normal]`, `[WindowShade]`, `[Equalizer]`, `[EqualizerWS]`) has a `NumPoints` list
//! and a `PointList`. `NumPoints` gives the vertex count of each polygon in the section
//! (a section may hold several polygons); `PointList` is the flat `x,y,x,y,...` stream
//! of vertices for those polygons in order.
//!
//! On Wayland these polygons become the window's input region (for click-through on the
//! transparent parts) plus a per-pixel edge test; here we only parse them into geometry.

use std::collections::HashMap;

pub type Point = (i32, i32);
pub type Polygon = Vec<Point>;

/// Parsed window shapes, one polygon list per window mode. Empty when a section is
/// absent (the window is then treated as its full rectangle).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Region {
    pub normal: Vec<Polygon>,
    pub window_shade: Vec<Polygon>,
    pub equalizer: Vec<Polygon>,
    pub equalizer_ws: Vec<Polygon>,
}

impl Region {
    pub fn parse(text: &str) -> Region {
        let mut section = String::new();
        let mut num_points: HashMap<String, Vec<usize>> = HashMap::new();
        let mut point_list: HashMap<String, Vec<i32>> = HashMap::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim().to_ascii_lowercase().as_str() {
                "numpoints" => {
                    let e = num_points.entry(section.clone()).or_default();
                    e.extend(v.split(',').filter_map(|t| t.trim().parse::<usize>().ok()));
                }
                "pointlist" => {
                    let e = point_list.entry(section.clone()).or_default();
                    e.extend(v.split(',').filter_map(|t| t.trim().parse::<i32>().ok()));
                }
                _ => {}
            }
        }

        let build = |name: &str| assemble(num_points.get(name), point_list.get(name));
        Region {
            normal: build("normal"),
            window_shade: build("windowshade"),
            equalizer: build("equalizer"),
            equalizer_ws: build("equalizerws"),
        }
    }
}

/// Split the flat point stream into polygons according to the per-polygon vertex counts.
fn assemble(counts: Option<&Vec<usize>>, points: Option<&Vec<i32>>) -> Vec<Polygon> {
    let (Some(counts), Some(points)) = (counts, points) else {
        return Vec::new();
    };
    let mut verts = points.chunks_exact(2).map(|c| (c[0], c[1]));
    let mut polys = Vec::with_capacity(counts.len());
    for &count in counts {
        let poly: Polygon = verts.by_ref().take(count).collect();
        if !poly.is_empty() {
            polys.push(poly);
        }
    }
    polys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rectangle_and_equalizer() {
        let txt = "\
[Normal]
NumPoints=4
PointList=0,0,275,0,275,116,0,116

[Equalizer]
NumPoints=4
PointList=0,0,275,0,275,116,0,116
";
        let r = Region::parse(txt);
        assert_eq!(
            r.normal,
            vec![vec![(0, 0), (275, 0), (275, 116), (0, 116)]]
        );
        assert_eq!(r.equalizer.len(), 1);
        assert!(r.window_shade.is_empty());
        assert!(r.equalizer_ws.is_empty());
    }

    #[test]
    fn splits_multiple_polygons_by_numpoints() {
        // Two polygons in one section: a triangle then a quad.
        let txt = "\
[Normal]
NumPoints=3,4
PointList=0,0,10,0,10,10, 20,20,30,20,30,30,20,30
";
        let r = Region::parse(txt);
        assert_eq!(r.normal.len(), 2);
        assert_eq!(r.normal[0], vec![(0, 0), (10, 0), (10, 10)]);
        assert_eq!(r.normal[1], vec![(20, 20), (30, 20), (30, 30), (20, 30)]);
    }

    #[test]
    fn missing_sections_are_empty() {
        let r = Region::parse("");
        assert_eq!(r, Region::default());
    }
}
