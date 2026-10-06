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
    let bands = dec.get_tag_u32(Tag::SamplesPerPixel).unwrap_or(1);
    if bands > 1 {
        return Err(format!(
            "the service returned a {bands}-band colour image instead of elevation values"
        ));
    }
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

/// Reads an Arc/Info ASCII Grid (`ncols`, `nrows`, `xllcorner|xllcenter`,
/// `yllcorner|yllcenter`, `cellsize` or `dx`/`dy`, optional `nodata_value`,
/// then rows from north to south).
///
/// Servers for EPSG:2180 disagree on whether the grid's "x" is easting or
/// northing. `expected` (min E, min N, max E, max N) is the requested box: the
/// lower-left corner is matched against it and a northing-first grid is
/// transposed into the usual north-up, easting-columns layout.
pub fn read_aaigrid(bytes: &[u8], expected: (f64, f64, f64, f64)) -> Result<Grid, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "ASCII grid is not valid UTF-8".to_string())?;
    let mut header: std::collections::HashMap<String, f64> = Default::default();
    let mut rest = text;
    loop {
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let line = rest[..line_end].trim();
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value), None) = (parts.next(), parts.next(), parts.next()) else {
            break;
        };
        if !key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            break;
        }
        let v: f64 = value
            .parse()
            .map_err(|_| format!("bad ASCII grid header value {line:?}"))?;
        header.insert(key.to_ascii_lowercase(), v);
        rest = &rest[(line_end + 1).min(rest.len())..];
    }
    let get = |k: &str| header.get(k).copied();
    let ncols = get("ncols").ok_or("ASCII grid without ncols")? as usize;
    let nrows = get("nrows").ok_or("ASCII grid without nrows")? as usize;
    let (dx, dy) = match (get("cellsize"), get("dx"), get("dy")) {
        (Some(c), _, _) => (c, c),
        (None, Some(dx), Some(dy)) => (dx, dy),
        _ => return Err("ASCII grid without cellsize".into()),
    };
    let x_ll = get("xllcorner")
        .or_else(|| get("xllcenter").map(|c| c - dx / 2.0))
        .ok_or("ASCII grid without xllcorner")?;
    let y_ll = get("yllcorner")
        .or_else(|| get("yllcenter").map(|c| c - dy / 2.0))
        .ok_or("ASCII grid without yllcorner")?;
    let nodata = get("nodata_value").map(|v| v as f32);

    let values: Vec<f32> = rest
        .split_whitespace()
        .map(|v| {
            v.parse::<f32>()
                .map_err(|_| format!("bad ASCII grid value {v:?}"))
        })
        .collect::<Result<_, _>>()?;
    if values.len() < ncols * nrows {
        return Err(format!(
            "ASCII grid has {} values, expected {}",
            values.len(),
            ncols * nrows
        ));
    }

    let (min_e, min_n, _, _) = expected;
    let normal = (x_ll - min_e).abs() + (y_ll - min_n).abs();
    let swapped = (x_ll - min_n).abs() + (y_ll - min_e).abs();
    if normal <= swapped {
        return Ok(Grid {
            width: ncols,
            height: nrows,
            left: x_ll,
            top: y_ll + nrows as f64 * dy,
            px: dx,
            py: dy,
            data: values[..ncols * nrows].to_vec(),
            nodata,
        });
    }
    // Northing-first grid: columns run north, rows run east (row 0 = max E).
    let (w, h) = (nrows, ncols);
    let mut data = vec![f32::NAN; w * h];
    for row in 0..nrows {
        for col in 0..ncols {
            let (r, c) = (ncols - 1 - col, nrows - 1 - row);
            data[r * w + c] = values[row * ncols + col];
        }
    }
    Ok(Grid {
        width: w,
        height: h,
        left: y_ll,
        top: x_ll + ncols as f64 * dx,
        px: dy,
        py: dx,
        data,
        nodata,
    })
}

/// Splits a MIME multipart body (WCS 2.0 servers often answer GetCoverage
/// with `multipart/related`: a GML description plus the data part). The
/// boundary is taken from the first line (`--boundary`). Returns the part
/// bodies, or `None` if the body is not multipart.
pub fn multipart_parts(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    // Servers may send a preamble line break (or a BOM) before the first boundary.
    let skip = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace() && *b != 0xEF && *b != 0xBB && *b != 0xBF)
        .unwrap_or(bytes.len());
    let bytes = &bytes[skip..];
    if !bytes.starts_with(b"--") {
        return None;
    }
    let first_line_end = bytes.iter().position(|&b| b == b'\n')?;
    let boundary = trim_ascii(&bytes[..first_line_end]).to_vec();
    if boundary.len() < 3 {
        return None;
    }
    let mut parts = Vec::new();
    let mut pos = first_line_end + 1;
    while pos < bytes.len() {
        let next = find(&bytes[pos..], &boundary).map(|i| pos + i);
        let end = next.unwrap_or(bytes.len());
        let part = &bytes[pos..end];
        // Headers end at the first blank line.
        let body_start = find(part, b"\r\n\r\n")
            .map(|i| i + 4)
            .or_else(|| find(part, b"\n\n").map(|i| i + 2));
        if let Some(start) = body_start {
            let headers = String::from_utf8_lossy(&part[..start]).to_ascii_lowercase();
            let mut body = &part[start..];
            // Drop the line break that precedes the next boundary.
            if body.ends_with(b"\r\n") {
                body = &body[..body.len() - 2];
            } else if body.ends_with(b"\n") {
                body = &body[..body.len() - 1];
            }
            // Data parts (image/*) first, descriptions (XML) after.
            if headers.contains("content-type: image/") {
                parts.insert(0, body);
            } else {
                parts.push(body);
            }
        }
        match next {
            Some(n) => {
                pos = n + boundary.len();
                if bytes[pos..].starts_with(b"--") {
                    break; // closing boundary
                }
                pos += bytes[pos..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(0, |i| i + 1);
            }
            None => break,
        }
    }
    Some(parts)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &b[start..end]
}

/// Reads an elevation grid: GeoTIFF or Arc/Info ASCII Grid, possibly wrapped
/// in a WCS multipart response (the first part that parses wins).
pub fn read_grid(bytes: &[u8], expected: (f64, f64, f64, f64)) -> Result<Grid, String> {
    if let Some(parts) = multipart_parts(bytes) {
        let mut last_err = "multipart response without parts".to_string();
        for part in parts {
            match read_single_grid(part, expected) {
                Ok(g) => return Ok(g),
                Err(e) => last_err = e,
            }
        }
        return Err(format!(
            "no elevation grid in the multipart response ({last_err})"
        ));
    }
    read_single_grid(bytes, expected)
}

fn read_single_grid(bytes: &[u8], expected: (f64, f64, f64, f64)) -> Result<Grid, String> {
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        read_geotiff(bytes)
    } else if bytes
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(5)
        .map(|b| b.to_ascii_lowercase())
        .eq(b"ncols".iter().copied())
    {
        read_aaigrid(bytes, expected)
    } else {
        let head: Vec<String> = bytes.iter().take(8).map(|b| format!("{b:02x}")).collect();
        Err(format!(
            "the response is neither a GeoTIFF nor an ASCII grid (first bytes: {})",
            head.join(" ")
        ))
    }
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
    fn colour_images_are_rejected() {
        use tiff::encoder::{TiffEncoder, colortype::RGB8};
        let mut buf = Cursor::new(Vec::new());
        TiffEncoder::new(&mut buf)
            .unwrap()
            .write_image::<RGB8>(2, 1, &[1, 2, 3, 4, 5, 6])
            .unwrap();
        let err = read_geotiff(&buf.into_inner()).unwrap_err();
        assert!(err.contains("colour image"), "{err}");
    }

    #[test]
    fn ascii_grid_easting_first() {
        let text = b"ncols 3\nnrows 2\nxllcorner 1000\nyllcorner 2000\ncellsize 1\nNODATA_value -9999\n1 2 3\n4 5 -9999\n";
        let g = read_grid(text, (1000.0, 2000.0, 1003.0, 2002.0)).unwrap();
        assert_eq!((g.width, g.height, g.left, g.top), (3, 2, 1000.0, 2002.0));
        assert_eq!(g.at(1000.5, 2001.5), Some(1.0)); // top-left
        assert_eq!(g.at(1001.5, 2000.5), Some(5.0)); // bottom-middle
        assert_eq!(g.at(1002.5, 2000.5), None); // nodata
    }

    #[test]
    fn ascii_grid_northing_first_is_transposed() {
        // Same 3 (E) × 2 (N) area served with x = northing, y = easting:
        // 2 columns (north), 3 rows (east, first row = easternmost).
        // Value = 10 * east_index + north_index (from SW).
        let text = b"ncols 2\nnrows 3\nxllcenter 2000.5\nyllcenter 1000.5\ncellsize 1\n20 21\n10 11\n0 1\n";
        let g = read_grid(text, (1000.0, 2000.0, 1003.0, 2002.0)).unwrap();
        assert_eq!((g.width, g.height, g.left, g.top), (3, 2, 1000.0, 2002.0));
        for e in 0..3 {
            for n in 0..2 {
                let v = g.at(1000.5 + e as f64, 2000.5 + n as f64).unwrap();
                assert_eq!(v, (10 * e + n) as f32, "e={e} n={n}");
            }
        }
    }

    #[test]
    fn wcs_multipart_responses_are_unpacked() {
        let body = b"--wcs\r\nContent-Type: text/xml\r\nContent-ID: wcs\r\n\r\n<gml:RectifiedGridCoverage/>\r\n--wcs\r\nContent-Type: image/x-aaigrid\r\nContent-Description: coverage data\r\nContent-Transfer-Encoding: binary\r\nContent-ID: coverage/out.asc\r\n\r\nncols 2\nnrows 1\nxllcorner 1000\nyllcorner 2000\ncellsize 1\n7 8\n\r\n--wcs--\r\n";
        let parts = multipart_parts(body).unwrap();
        assert_eq!(parts.len(), 2);
        // The image/* part comes first, the XML description after it.
        assert!(parts[0].starts_with(b"ncols"));
        assert_eq!(parts[1], b"<gml:RectifiedGridCoverage/>");
        let g = read_grid(body, (1000.0, 2000.0, 1002.0, 2001.0)).unwrap();
        assert_eq!(g.at(1001.5, 2000.5), Some(8.0));

        // Bare \n line endings and a GeoTIFF part.
        let tif = encode_geotiff(&grid(0.0, 1.0, 1, 1, 1.0, |_, _| 5.0));
        let mut body = b"--b\nContent-Type: image/tiff\n\n".to_vec();
        body.extend_from_slice(&tif);
        body.extend_from_slice(b"\n--b--\n");
        assert_eq!(
            read_grid(&body, (0.0, 0.0, 1.0, 1.0)).unwrap().at(0.5, 0.5),
            Some(5.0)
        );

        let err = read_grid(
            b"--x\nContent-Type: text/xml\n\n<a/>\n--x--\n",
            (0.0, 0.0, 1.0, 1.0),
        )
        .unwrap_err();
        assert!(err.contains("no elevation grid"), "{err}");
        assert!(multipart_parts(b"ncols 1").is_none());

        // Preamble line break before the first boundary; data part listed
        // second in the body but tried first.
        let body = b"\r\n--wcs\r\nContent-Type: text/xml\r\n\r\n<gml/>\r\n--wcs\r\nContent-Type: image/x-aaigrid\r\n\r\nncols 1\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n9\r\n--wcs--\r\n";
        let parts = multipart_parts(body).unwrap();
        assert!(parts[0].starts_with(b"ncols"));
        assert_eq!(
            read_grid(body, (0.0, 0.0, 1.0, 1.0)).unwrap().at(0.5, 0.5),
            Some(9.0)
        );
    }

    #[test]
    fn unknown_formats_are_rejected() {
        assert!(read_grid(b"<ServiceException/>", (0.0, 0.0, 1.0, 1.0)).is_err());
        assert!(
            read_grid(
                b"ncols 2\nnrows 1\nxllcorner 0\nyllcorner 0\ncellsize 1\n1\n",
                (0.0, 0.0, 2.0, 1.0)
            )
            .is_err()
        );
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
