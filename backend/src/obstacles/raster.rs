//! Elevation grids (GeoTIFF) and conversion of a surface model minus a terrain
//! model into obstacle blocks.

use std::io::Cursor;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

const TAG_MODEL_PIXEL_SCALE: u16 = 33550;
const TAG_MODEL_TIEPOINT: u16 = 33922;
const TAG_GDAL_NODATA: u16 = 42113;

/// A north-up elevation grid in projected metres (here: EPSG:2180).
#[derive(Debug, Clone)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// Easting of the left edge of the first column.
    pub left: f64,
    /// Northing of the top edge of the first row.
    pub top: f64,
    /// Pixel size (metres) along easting and northing.
    pub px: f64,
    pub py: f64,
    pub data: Vec<f32>,
    pub nodata: Option<f32>,
}

impl Grid {
    /// Value of the pixel containing (easting, northing), if any.
    pub fn at(&self, e: f64, n: f64) -> Option<f32> {
        let c = ((e - self.left) / self.px).floor();
        let r = ((self.top - n) / self.py).floor();
        if c < 0.0 || r < 0.0 || c >= self.width as f64 || r >= self.height as f64 {
            return None;
        }
        let v = self.data[r as usize * self.width + c as usize];
        let is_nodata = self.nodata.is_some_and(|nd| (v - nd).abs() < 1e-3);
        (v.is_finite() && !is_nodata && v > -1000.0).then_some(v)
    }
}

/// Reads a single-band GeoTIFF with ModelPixelScale + ModelTiepoint tags.
pub fn read_geotiff(bytes: &[u8]) -> Result<Grid, String> {
    let mut dec = Decoder::new(Cursor::new(bytes)).map_err(|e| format!("not a TIFF: {e}"))?;
    let (w, h) = dec.dimensions().map_err(|e| e.to_string())?;
    let scale = dec
        .get_tag_f64_vec(Tag::Unknown(TAG_MODEL_PIXEL_SCALE))
        .map_err(|_| "GeoTIFF has no ModelPixelScale tag".to_string())?;
    let tie = dec
        .get_tag_f64_vec(Tag::Unknown(TAG_MODEL_TIEPOINT))
        .map_err(|_| "GeoTIFF has no ModelTiepoint tag".to_string())?;
    if scale.len() < 2 || tie.len() < 6 {
        return Err("malformed GeoTIFF georeference".into());
    }
    let nodata = dec
        .get_tag_ascii_string(Tag::Unknown(TAG_GDAL_NODATA))
        .ok()
        .and_then(|s| s.trim().trim_end_matches('\0').parse::<f32>().ok());
    let data: Vec<f32> = match dec.read_image().map_err(|e| e.to_string())? {
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::U16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => return Err("unsupported GeoTIFF sample format".into()),
    };
    let (w, h) = (w as usize, h as usize);
    if data.len() < w * h {
        return Err("GeoTIFF has more than one band or truncated data".into());
    }
    // Tiepoint maps raster (i, j) to model (x, y).
    let (i, j, x, y) = (tie[0], tie[1], tie[3], tie[4]);
    Ok(Grid {
        width: w,
        height: h,
        left: x - i * scale[0],
        top: y + j * scale[1],
        px: scale[0],
        py: scale[1],
        data: data[..w * h].to_vec(),
        nodata,
    })
}

/// Writes a single-band float GeoTIFF the way GDAL / WCS servers do
/// (pixel scale + tiepoint). Used for tests and fixtures.
pub fn encode_geotiff(g: &Grid) -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype::Gray32Float};
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut buf).unwrap();
        let mut img = enc
            .new_image::<Gray32Float>(g.width as u32, g.height as u32)
            .unwrap();
        img.encoder()
            .write_tag(Tag::Unknown(TAG_MODEL_PIXEL_SCALE), &[g.px, g.py, 0.0][..])
            .unwrap();
        img.encoder()
            .write_tag(
                Tag::Unknown(TAG_MODEL_TIEPOINT),
                &[0.0, 0.0, 0.0, g.left, g.top, 0.0][..],
            )
            .unwrap();
        if let Some(nd) = g.nodata {
            img.encoder()
                .write_tag(Tag::Unknown(TAG_GDAL_NODATA), &*format!("{nd}"))
                .unwrap();
        }
        img.write_data(&g.data).unwrap();
    }
    buf.into_inner()
}

/// An axis-aligned block (in projected metres) of roughly constant height.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
    /// Height of the object above the local ground (NMPT − NMT), metres.
    pub height: f64,
    /// Ground level relative to the reference (take-off) ground, metres.
    pub ground_rel: f64,
}

pub struct BlockParams {
    pub cell_m: f64,
    /// Cells lower than this above ground are ignored (grass, cars, noise).
    pub min_height_m: f64,
    /// Ground elevation used as zero for `ground_rel`.
    pub reference_ground: f64,
    pub max_blocks: usize,
}

/// Turns surface (DSM) and terrain (DTM) grids into obstacle blocks over the
/// area [west, east] × [south, north]. Each cell takes the highest surface
/// sample inside it; runs of neighbouring cells in a row with the same
/// height (1 m steps) and ground (1 m) are merged into one block.
pub fn blocks(
    dsm: &Grid,
    dtm: &Grid,
    (west, south, east, north): (f64, f64, f64, f64),
    p: &BlockParams,
) -> Vec<Block> {
    let cols = ((east - west) / p.cell_m).ceil().max(0.0) as usize;
    let rows = ((north - south) / p.cell_m).ceil().max(0.0) as usize;
    // Sub-samples per cell side, so a cell sees every DSM pixel inside it.
    let sub = (p.cell_m / dsm.px.min(dsm.py)).ceil().clamp(1.0, 8.0) as usize;
    let mut out: Vec<Block> = Vec::new();

    for r in 0..rows {
        let n1 = north - r as f64 * p.cell_m;
        let n0 = n1 - p.cell_m;
        let mut run: Option<(usize, i64, i64)> = None; // (start col, height q, ground q)
        let flush = |run: &mut Option<(usize, i64, i64)>, end: usize, out: &mut Vec<Block>| {
            if let Some((c0, hq, gq)) = run.take() {
                out.push(Block {
                    west: west + c0 as f64 * p.cell_m,
                    east: west + end as f64 * p.cell_m,
                    south: n0,
                    north: n1,
                    height: hq as f64,
                    ground_rel: gq as f64,
                });
            }
        };
        for c in 0..=cols {
            let cell = (c < cols).then(|| {
                let e0 = west + c as f64 * p.cell_m;
                let mut top = f32::NEG_INFINITY;
                for i in 0..sub {
                    for j in 0..sub {
                        let e = e0 + (i as f64 + 0.5) * p.cell_m / sub as f64;
                        let n = n0 + (j as f64 + 0.5) * p.cell_m / sub as f64;
                        if let Some(v) = dsm.at(e, n) {
                            top = top.max(v);
                        }
                    }
                }
                let ground = dtm.at(e0 + p.cell_m / 2.0, n0 + p.cell_m / 2.0)?;
                let h = top as f64 - ground as f64;
                (top.is_finite() && h >= p.min_height_m).then(|| {
                    (
                        h.round() as i64,
                        (ground as f64 - p.reference_ground).round() as i64,
                    )
                })
            });
            match (cell.flatten(), &mut run) {
                (Some((hq, gq)), Some((_, rh, rg))) if *rh == hq && *rg == gq => {}
                (Some((hq, gq)), _) => {
                    flush(&mut run, c, &mut out);
                    run = Some((c, hq, gq));
                }
                (None, _) => flush(&mut run, c, &mut out),
            }
        }
        if out.len() > p.max_blocks {
            out.truncate(p.max_blocks);
            break;
        }
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn grid(
        left: f64,
        top: f64,
        w: usize,
        h: usize,
        px: f64,
        f: impl Fn(usize, usize) -> f32,
    ) -> Grid {
        let mut data = Vec::with_capacity(w * h);
        for r in 0..h {
            for c in 0..w {
                data.push(f(c, r));
            }
        }
        Grid {
            width: w,
            height: h,
            left,
            top,
            px,
            py: px,
            data,
            nodata: Some(-9999.0),
        }
    }

    #[test]
    fn geotiff_round_trip_and_lookup() {
        let g = grid(1000.0, 2000.0, 4, 3, 0.5, |c, r| (r * 10 + c) as f32);
        let back = read_geotiff(&encode_geotiff(&g)).unwrap();
        assert_eq!((back.width, back.height), (4, 3));
        assert_eq!((back.left, back.top, back.px), (1000.0, 2000.0, 0.5));
        assert_eq!(back.nodata, Some(-9999.0));
        // Pixel (col 2, row 1) covers e 1001.0..1001.5, n 1999.0..1999.5.
        assert_eq!(back.at(1001.2, 1999.2), Some(12.0));
        assert_eq!(back.at(999.0, 1999.2), None);
        assert!(read_geotiff(b"not a tiff").is_err());
    }

    #[test]
    fn nodata_is_ignored() {
        let g = grid(
            0.0,
            10.0,
            2,
            1,
            1.0,
            |c, _| if c == 0 { -9999.0 } else { 5.0 },
        );
        assert_eq!(g.at(0.5, 9.5), None);
        assert_eq!(g.at(1.5, 9.5), Some(5.0));
    }

    #[test]
    fn surface_minus_terrain_becomes_merged_blocks() {
        // 20 × 10 m area, ground sloping by 0 m (flat at 100 m).
        let dtm = grid(0.0, 10.0, 20, 10, 1.0, |_, _| 100.0);
        // A 12 m "house" at e 4..8, n 2..6 and a 20 m "tree" at e 14..16, n 4..6.
        let dsm = grid(0.0, 10.0, 40, 20, 0.5, |c, r| {
            let (e, n) = (c as f64 * 0.5 + 0.25, 10.0 - (r as f64 * 0.5 + 0.25));
            if (4.0..8.0).contains(&e) && (2.0..6.0).contains(&n) {
                112.0
            } else if (14.0..16.0).contains(&e) && (4.0..6.0).contains(&n) {
                120.0
            } else {
                100.3 // grass: below min height
            }
        });
        let p = BlockParams {
            cell_m: 2.0,
            min_height_m: 2.5,
            reference_ground: 98.0,
            max_blocks: 1000,
        };
        let b = blocks(&dsm, &dtm, (0.0, 0.0, 20.0, 10.0), &p);
        // House: 2 rows of one merged 4 m block; tree: one 2 × 2 m block.
        let house: Vec<_> = b.iter().filter(|x| x.height == 12.0).collect();
        let tree: Vec<_> = b.iter().filter(|x| x.height == 20.0).collect();
        assert_eq!(house.len(), 2, "{b:?}");
        assert!(house.iter().all(|x| x.west == 4.0 && x.east == 8.0));
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (tree[0].west, tree[0].east, tree[0].south, tree[0].north),
            (14.0, 16.0, 4.0, 6.0)
        );
        assert!(b.iter().all(|x| x.ground_rel == 2.0));
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn block_count_is_capped() {
        let dtm = grid(0.0, 100.0, 100, 100, 1.0, |_, _| 0.0);
        // Checkerboard of 10 m spikes: nothing merges.
        let dsm = grid(0.0, 100.0, 100, 100, 1.0, |c, r| {
            if (c / 2 + r / 2) % 2 == 0 { 10.0 } else { 0.0 }
        });
        let p = BlockParams {
            cell_m: 2.0,
            min_height_m: 2.5,
            reference_ground: 0.0,
            max_blocks: 50,
        };
        assert!(blocks(&dsm, &dtm, (0.0, 0.0, 100.0, 100.0), &p).len() <= 50);
    }
}
