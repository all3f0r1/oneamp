//! Parser for Winamp `region.txt` skin files.
//!
//! Format (one or more sections, e.g. `[Normal]`, `[WindowShade]`,
//! `[Equalizer]`, `[EqualizerWS]`):
//!
//! ```text
//! [Normal]
//! NumPoints = 4, 4, 4
//! PointList = 5,0 270,0 270,115 5,115  3,1 272,1 272,114 3,114  ...
//! ```
//!
//! `NumPoints` is a comma-separated list of polygon vertex counts. The
//! `PointList` is one long string of `x,y` pairs (whitespace-separated)
//! consumed in order to fill those polygons. A region is the *union* of all
//! its polygons — a pixel is "inside the region" if it's inside at least one
//! of them.
//!
//! Lines beginning with `;` or `#` are comments. Comments after data on the
//! same line are also stripped (`;` only — Winamp's own region files use
//! it).

use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionPoint {
    pub x: i32,
    pub y: i32,
}

impl RegionPoint {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Polygon {
    pub points: Vec<RegionPoint>,
}

impl Polygon {
    /// Even-odd ray-cast point-in-polygon test. Vertices on the boundary
    /// are treated inclusively for the top/left edge by convention; this is
    /// good enough for skin masks where pixel coordinates are integers.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let n = self.points.len();
        if n < 3 {
            return false;
        }

        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let pi = self.points[i];
            let pj = self.points[j];
            if (pi.y > y) != (pj.y > y) {
                let dx = (pj.x - pi.x) as i64;
                let dy = (pj.y - pi.y) as i64;
                if dy != 0 {
                    let intersect_x = pi.x as i64 + dx * (y - pi.y) as i64 / dy;
                    if (x as i64) < intersect_x {
                        inside = !inside;
                    }
                }
            }
            j = i;
        }
        inside
    }
}

#[derive(Debug, Clone, Default)]
pub struct Region {
    pub name: String,
    pub polygons: Vec<Polygon>,
}

impl Region {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            polygons: Vec::new(),
        }
    }

    /// Region is empty when no polygon has been declared. Callers should
    /// treat an empty region as "no mask" (full window opaque), since
    /// classic Winamp skins commonly ship a `region.txt` that's entirely
    /// commented out — meaning "use the default rectangular shape".
    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    /// True when at least one of the region's polygons contains the point.
    /// The region is the *union* of its polygons.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.polygons.iter().any(|p| p.contains(x, y))
    }
}

/// Strip a `;`-prefixed trailing comment from a line, then trim whitespace.
fn strip_comment(line: &str) -> &str {
    let cut = line.find(';').unwrap_or(line.len());
    line[..cut].trim()
}

#[derive(Default)]
struct Builder {
    name: String,
    num_points: Vec<usize>,
    points: Vec<RegionPoint>,
}

impl Builder {
    fn new(name: String) -> Self {
        Self {
            name,
            num_points: Vec::new(),
            points: Vec::new(),
        }
    }

    fn finish(self) -> Region {
        let mut region = Region::new(self.name);
        if self.num_points.is_empty() {
            // No NumPoints declared — accept the whole point list as a
            // single polygon if non-empty.
            if !self.points.is_empty() {
                region.polygons.push(Polygon {
                    points: self.points,
                });
            }
            return region;
        }

        let mut idx = 0;
        for &count in &self.num_points {
            if count == 0 || idx + count > self.points.len() {
                break;
            }
            let poly_points = self.points[idx..idx + count].to_vec();
            region.polygons.push(Polygon {
                points: poly_points,
            });
            idx += count;
        }
        region
    }
}

fn parse_point_list(value: &str, points: &mut Vec<RegionPoint>) {
    for pair in value.split_whitespace() {
        let mut it = pair.split(',');
        if let (Some(xs), Some(ys)) = (it.next(), it.next())
            && let (Ok(x), Ok(y)) = (xs.trim().parse::<i32>(), ys.trim().parse::<i32>())
        {
            points.push(RegionPoint::new(x, y));
        }
    }
}

pub fn parse_region_file(content: &str) -> Result<Vec<Region>> {
    let mut regions: Vec<Region> = Vec::new();
    let mut current: Option<Builder> = None;

    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let line = strip_comment(trimmed);
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            if let Some(b) = current.take() {
                regions.push(b.finish());
            }
            let name = line[1..line.len() - 1].trim().to_string();
            current = Some(Builder::new(name));
            continue;
        }

        let Some(eq_pos) = line.find('=') else {
            continue;
        };
        let key = line[..eq_pos].trim().to_ascii_lowercase();
        let value = line[eq_pos + 1..].trim();

        let Some(builder) = current.as_mut() else {
            continue;
        };

        match key.as_str() {
            "numpoints" => {
                builder.num_points = value
                    .split(',')
                    .filter_map(|s| s.trim().parse::<usize>().ok())
                    .collect();
            }
            "pointlist" => {
                parse_point_list(value, &mut builder.points);
            }
            _ => {}
        }
    }

    if let Some(b) = current.take() {
        regions.push(b.finish());
    }

    Ok(regions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_inside_outside() {
        let p = Polygon {
            points: vec![
                RegionPoint::new(0, 0),
                RegionPoint::new(10, 0),
                RegionPoint::new(10, 10),
                RegionPoint::new(0, 10),
            ],
        };
        assert!(p.contains(5, 5));
        assert!(!p.contains(15, 5));
        assert!(!p.contains(-1, 5));
    }

    #[test]
    fn region_union_of_polygons() {
        let mut r = Region::new("Normal");
        r.polygons.push(Polygon {
            points: vec![
                RegionPoint::new(0, 0),
                RegionPoint::new(5, 0),
                RegionPoint::new(5, 5),
                RegionPoint::new(0, 5),
            ],
        });
        r.polygons.push(Polygon {
            points: vec![
                RegionPoint::new(10, 10),
                RegionPoint::new(20, 10),
                RegionPoint::new(20, 20),
                RegionPoint::new(10, 20),
            ],
        });
        assert!(r.contains(2, 2));
        assert!(r.contains(15, 15));
        assert!(!r.contains(7, 7));
    }

    #[test]
    fn parse_winamp_default_normal() {
        // Real example pulled from the Winamp default skin's region.txt.
        // 7 polygons of 4 points each, sharing one PointList line.
        let content = r#"
[Normal]
NumPoints=4,4,4,4,4,4,4
PointList=5,0 270,0 270,115 5,115 3,1 272,1 272,114 3,114 2,2 273,2 273,113 2,113 1,3 274,3 274,112 1,112 0,5 275,5 275,110 0,110 4,114 271,114 271,115 4,115 6,115 269,115 269,116 6,116
"#;
        let regions = parse_region_file(content).unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].name, "Normal");
        assert_eq!(regions[0].polygons.len(), 7);
        for poly in &regions[0].polygons {
            assert_eq!(poly.points.len(), 4);
        }
        // Pixels well inside the window are covered by the union.
        assert!(regions[0].contains(100, 50));
        // Top corners are NOT covered (chamfered).
        assert!(!regions[0].contains(0, 0));
        assert!(!regions[0].contains(274, 0));
    }

    #[test]
    fn parse_multiple_sections() {
        let content = r#"
[Normal]
NumPoints=4
PointList=0,0 10,0 10,10 0,10

[Equalizer]
NumPoints=3
PointList=0,0 10,0 5,10
"#;
        let regions = parse_region_file(content).unwrap();
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].name, "Normal");
        assert_eq!(regions[1].name, "Equalizer");
        assert_eq!(regions[0].polygons[0].points.len(), 4);
        assert_eq!(regions[1].polygons[0].points.len(), 3);
    }

    #[test]
    fn parse_handles_inline_comments() {
        let content = r#"
[Normal] ; the main window
NumPoints=4 ; one square
PointList=0,0 10,0 10,10 0,10 ; corners
"#;
        let regions = parse_region_file(content).unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].polygons.len(), 1);
        assert_eq!(regions[0].polygons[0].points.len(), 4);
    }

    #[test]
    fn parse_skin_with_only_comments_yields_no_regions() {
        // base-2.91.wsz ships a region.txt that's entirely comments — the
        // skin is a plain rectangle. Empty Vec is the right answer.
        let content = r#"
; just a comment
; [Normal]
; NumPoints=4
"#;
        let regions = parse_region_file(content).unwrap();
        assert!(regions.is_empty());
    }

    #[test]
    fn empty_input_is_ok() {
        let regions = parse_region_file("").unwrap();
        assert!(regions.is_empty());
    }
}
