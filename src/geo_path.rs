//! Bepaalt het geografische pad op basis van contacten.
//!
//! Als **eigen positie** (`self_pos`, lon/lat) bekend is: het pad wordt **vanaf de lokale radio**
//! opgebouwd door de hop-rij **achterwaarts** te doorlopen — per hop de contact-GPS die het
//! **dichtst bij het vorige punt** ligt (eerst het dichtst bij jezelf, dan het dichtst bij het
//! zo gevonden punt, enz.). De uitvoer blijft in **packet-volgorde** (`coords[i]` bij `hops[i]`).
//!
//! Zonder eigen positie: valt terug op de eerdere voorwaartse inferentie (DP + relaxed synthese).
//!
//! **Direct bereik:** tussen opeenvolgende knooppunten op het pad wordt maximaal
//! [`MAX_DIRECT_HOP_KM`] km aangenomen (geen rechtstreekse RF-link verder dan dit).

use crate::contact_book::ContactBook;

/// Maximaal haversine-afstand tussen twee opeenvolgende knooppunten op het pad (km).
const MAX_DIRECT_HOP_KM: f64 = 60.0;

fn gps_points_for_hop(book: &ContactBook, hop_hex: &str) -> Vec<(f64, f64)> {
    let mut v = Vec::new();
    for c in book.contacts_for_hop_prefix(hop_hex) {
        if let (Some(lo), Some(la)) = (c.lon, c.lat) {
            v.push((lo, la));
        }
    }
    v
}

/// Eerste byte van de hop-hex (voor synthese-randomness).
fn hex_hop_tag_byte(hop_hex: &str) -> u8 {
    let h = hop_hex.trim().to_lowercase();
    if h.len() < 2 {
        return 0;
    }
    if h.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Ok(bytes) = hex::decode(&h) {
            if let Some(&b) = bytes.first() {
                return b;
            }
        }
    }
    u8::from_str_radix(&h[..2], 16).unwrap_or(0)
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lon1, lat1) = (a.0.to_radians(), a.1.to_radians());
    let (lon2, lat2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * h.sqrt().asin() * 6371.0
}

/// Ruwe anker uit het pad zelf: gemiddelde van de eerste bekende GPS per hop (tie-break eerste segment).
fn path_anchor_from_book(book: &ContactBook, hops_hex: &[String]) -> (f64, f64) {
    let mut sum = (0.0_f64, 0.0_f64);
    let mut n = 0usize;
    for hop in hops_hex {
        for c in book.contacts_for_hop_prefix(hop) {
            if let (Some(lo), Some(la)) = (c.lon, c.lat) {
                sum.0 += lo;
                sum.1 += la;
                n += 1;
                break;
            }
        }
    }
    if n > 0 {
        (sum.0 / n as f64, sum.1 / n as f64)
    } else {
        (5.3, 51.7)
    }
}

/// Dichtstbijzijnde GPS-kandidaat bij `prev` die binnen `max_km` ligt; anders `None`.
fn nearest_gps_within_km(
    prev: (f64, f64),
    candidates: &[(f64, f64)],
    max_km: f64,
) -> Option<(f64, f64)> {
    candidates
        .iter()
        .copied()
        .filter(|p| haversine_km(prev, *p) <= max_km)
        .min_by(|a, b| {
            haversine_km(prev, *a)
                .partial_cmp(&haversine_km(prev, *b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn synth_step(prev: (f64, f64), hop_index: usize, b: u8) -> (f64, f64) {
    let angle = (hop_index as f64) * 0.35 + (b as f64) * 0.01;
    (
        prev.0 + angle.cos() * 0.12,
        prev.1 + angle.sin() * 0.08,
    )
}

/// Strikt **achterwaarts vanaf eigen positie**: elke hop moet minstens één GPS-kandidaat hebben.
fn infer_route_lon_lat_reverse_from_self_strict(
    hops_hex: &[String],
    book: &ContactBook,
    self_lon_lat: (f64, f64),
) -> Vec<(f64, f64)> {
    if hops_hex.is_empty() {
        return vec![];
    }
    let mut prev = self_lon_lat;
    let mut rev: Vec<(f64, f64)> = Vec::with_capacity(hops_hex.len());
    for hop_hex in hops_hex.iter().rev() {
        let gps = gps_points_for_hop(book, hop_hex);
        if gps.is_empty() {
            return vec![];
        }
        let Some(p) = nearest_gps_within_km(prev, &gps, MAX_DIRECT_HOP_KM) else {
            return vec![];
        };
        rev.push(p);
        prev = p;
    }
    rev.reverse();
    spread_degenerate_coords(&mut rev);
    rev
}

/// Relaxed achterwaarts vanaf eigen positie: synthese als een hop geen GPS heeft.
fn infer_route_lon_lat_reverse_from_self_relaxed(
    hops_hex: &[String],
    book: &ContactBook,
    self_lon_lat: (f64, f64),
) -> Vec<(f64, f64)> {
    if hops_hex.is_empty() {
        return vec![];
    }
    let mut prev = self_lon_lat;
    let mut rev: Vec<(f64, f64)> = Vec::with_capacity(hops_hex.len());
    for (rev_i, hop_hex) in hops_hex.iter().rev().enumerate() {
        let gps = gps_points_for_hop(book, hop_hex);
        let p = if let Some(q) = nearest_gps_within_km(prev, &gps, MAX_DIRECT_HOP_KM) {
            q
        } else {
            synth_step(prev, rev_i, hex_hop_tag_byte(hop_hex))
        };
        rev.push(p);
        prev = p;
    }
    rev.reverse();
    spread_degenerate_coords(&mut rev);
    rev
}

/// Strikt (historisch, voorwaarts): elke hop moet een bekend contact én minstens één GPS-kandidaat hebben.
fn infer_route_lon_lat_strict(hops_hex: &[String], book: &ContactBook) -> Vec<(f64, f64)> {
    if hops_hex.is_empty() {
        return vec![];
    }

    let mut gps_per_hop: Vec<Vec<(f64, f64)>> = Vec::with_capacity(hops_hex.len());
    for hop_hex in hops_hex {
        if book.contacts_for_hop_prefix(hop_hex).is_empty() {
            return vec![];
        }
        let gps = gps_points_for_hop(book, hop_hex);
        if gps.is_empty() {
            return vec![];
        }
        gps_per_hop.push(gps);
    }

    let anchor = path_anchor_from_book(book, hops_hex);
    let mut out = dp_segment(&gps_per_hop, None, anchor, true);
    spread_degenerate_coords(&mut out);
    out
}

/// Relaxed: synthetische punten als een hop onbekend is of geen GPS heeft; bij meerdere GPS-kandidaten
/// wordt de dichtstbijzijnde bij het vorige punt gekozen.
fn infer_route_lon_lat_relaxed(hops_hex: &[String], book: &ContactBook) -> Vec<(f64, f64)> {
    if hops_hex.is_empty() {
        return vec![];
    }
    let anchor = path_anchor_from_book(book, hops_hex);
    let mut out = Vec::with_capacity(hops_hex.len());
    let mut prev = anchor;
    for (i, hop_hex) in hops_hex.iter().enumerate() {
        let gps = gps_points_for_hop(book, hop_hex);
        let p = if let Some(q) = nearest_gps_within_km(prev, &gps, MAX_DIRECT_HOP_KM) {
            q
        } else {
            synth_step(prev, i, hex_hop_tag_byte(hop_hex))
        };
        out.push(p);
        prev = p;
    }
    spread_degenerate_coords(&mut out);
    out
}

/// `hops_hex`: per hop hex-prefix uit het RF-pad (1–4 bytes → 2–8 hextekens), te matchen met het begin
/// van `ContactRecord::pubkey_prefix_hex`. Eén `(lon, lat)` per hop in packet-volgorde.
///
/// `self_pos`: `(lon, lat)` van de lokale radio (advert). Als gezet: **achterwaarts** dichtstbijzijnde
/// keten vanaf dit punt. Anders: voorwaartse fallback (DP + relaxed).
pub fn infer_route_lon_lat(
    hops_hex: &[String],
    book: &ContactBook,
    self_pos: Option<(f64, f64)>,
) -> Vec<(f64, f64)> {
    if let Some(s) = self_pos {
        let strict = infer_route_lon_lat_reverse_from_self_strict(hops_hex, book, s);
        if !strict.is_empty() {
            return strict;
        }
        return infer_route_lon_lat_reverse_from_self_relaxed(hops_hex, book, s);
    }
    let strict = infer_route_lon_lat_strict(hops_hex, book);
    if !strict.is_empty() {
        return strict;
    }
    infer_route_lon_lat_relaxed(hops_hex, book)
}

/// Viterbi op een doorlopende GPS-segment: minimaliseert som van segmentafstanden.
fn dp_segment(
    gps_rows: &[Vec<(f64, f64)>],
    prev_before_seg: Option<(f64, f64)>,
    path_anchor: (f64, f64),
    is_path_start: bool,
) -> Vec<(f64, f64)> {
    let len = gps_rows.len();
    if len == 0 {
        return vec![];
    }
    if len == 1 {
        let pts = &gps_rows[0];
        if pts.len() == 1 {
            let p = pts[0];
            let c = if is_path_start {
                haversine_km(path_anchor, p)
            } else if let Some(prev) = prev_before_seg {
                haversine_km(prev, p)
            } else {
                0.0
            };
            return if c <= MAX_DIRECT_HOP_KM {
                vec![p]
            } else {
                vec![]
            };
        }
        let mut best_i: Option<usize> = None;
        let mut best_c = f64::MAX;
        for (i, &p) in pts.iter().enumerate() {
            let c = if is_path_start {
                haversine_km(path_anchor, p)
            } else if let Some(prev) = prev_before_seg {
                haversine_km(prev, p)
            } else {
                0.0
            };
            if c <= MAX_DIRECT_HOP_KM && c < best_c {
                best_c = c;
                best_i = Some(i);
            }
        }
        return best_i.map(|i| vec![pts[i]]).unwrap_or_default();
    }

    let mut dp: Vec<Vec<f64>> = (0..len).map(|_| vec![]).collect();
    let mut parent: Vec<Vec<usize>> = (0..len).map(|_| vec![]).collect();

    let n0 = gps_rows[0].len();
    dp[0] = vec![0.0; n0];
    for (j, &p) in gps_rows[0].iter().enumerate() {
        let dist = if is_path_start {
            haversine_km(path_anchor, p)
        } else if let Some(prev) = prev_before_seg {
            haversine_km(prev, p)
        } else {
            0.0
        };
        dp[0][j] = if dist <= MAX_DIRECT_HOP_KM {
            dist
        } else {
            f64::INFINITY
        };
    }

    for i in 1..len {
        let ni = gps_rows[i].len();
        let ni_prev = gps_rows[i - 1].len();
        dp[i] = vec![f64::INFINITY; ni];
        parent[i] = vec![0; ni];
        for k in 0..ni {
            let pk = gps_rows[i][k];
            for j in 0..ni_prev {
                let pj = gps_rows[i - 1][j];
                let seg = haversine_km(pj, pk);
                if seg > MAX_DIRECT_HOP_KM {
                    continue;
                }
                let d = dp[i - 1][j] + seg;
                if d < dp[i][k] {
                    dp[i][k] = d;
                    parent[i][k] = j;
                }
            }
        }
    }

    let last = len - 1;
    let mut best_j = 0usize;
    let mut best_d = f64::INFINITY;
    for j in 0..gps_rows[last].len() {
        if dp[last][j] < best_d {
            best_d = dp[last][j];
            best_j = j;
        }
    }
    if !best_d.is_finite() {
        return vec![];
    }

    let mut chain = vec![0usize; len];
    chain[last] = best_j;
    for i in (1..len).rev() {
        chain[i - 1] = parent[i][chain[i]];
    }

    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(gps_rows[i][chain[i]]);
    }
    out
}

/// Voorkomt een kaartlijn op één breedtegraad (of meridiaan) door foute of identieke punten.
fn spread_degenerate_coords(coords: &mut Vec<(f64, f64)>) {
    if coords.len() < 2 {
        return;
    }
    let lat_min = coords.iter().map(|(_, la)| *la).fold(f64::INFINITY, f64::min);
    let lat_max = coords.iter().map(|(_, la)| *la).fold(f64::NEG_INFINITY, f64::max);
    let lon_min = coords.iter().map(|(lo, _)| *lo).fold(f64::INFINITY, f64::min);
    let lon_max = coords.iter().map(|(lo, _)| *lo).fold(f64::NEG_INFINITY, f64::max);
    let lat_span = lat_max - lat_min;
    let lon_span = lon_max - lon_min;
    const EPS: f64 = 1e-7;
    if lat_span < EPS && lon_span > EPS {
        for (i, c) in coords.iter_mut().enumerate() {
            c.1 += (i as f64) * 0.002 + ((i as f64) * 0.7).sin() * 0.01;
        }
    } else if lon_span < EPS && lat_span > EPS {
        for (i, c) in coords.iter_mut().enumerate() {
            c.0 += (i as f64) * 0.002 + ((i as f64) * 0.7).sin() * 0.01;
        }
    } else if lat_span < EPS && lon_span < EPS {
        for (i, c) in coords.iter_mut().enumerate() {
            let t = i as f64;
            c.0 += t.cos() * 0.02;
            c.1 += t.sin() * 0.02;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_book::{ContactBook, ContactRecord};

    #[test]
    fn empty_hops() {
        let book = ContactBook::default();
        assert!(infer_route_lon_lat(&[], &book, None).is_empty());
    }

    #[test]
    fn dp_prefers_shorter_chain() {
        let mut book = ContactBook::default();
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab0001".into(),
            name: "A".into(),
            lat: Some(52.0),
            lon: Some(5.0),
        });
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab0002".into(),
            name: "B".into(),
            lat: Some(53.0),
            lon: Some(5.0),
        });
        book.upsert(ContactRecord {
            hash0: 0xcd,
            pubkey_prefix_hex: "cd0001".into(),
            name: "C".into(),
            lat: Some(52.05),
            lon: Some(5.05),
        });
        let hops = vec!["ab".into(), "cd".into()];
        // Eigen positie gelijk aan station A — achterwaarts: eerst `cd`→C dichtst bij self, dan `ab`→dichtst bij C.
        let self_pos = (5.0_f64, 52.0_f64);
        let v = infer_route_lon_lat(&hops, &book, Some(self_pos));
        assert_eq!(v.len(), 2);
        let dist_to_a = haversine_km((5.0, 52.0), v[0]);
        let dist_to_b = haversine_km((5.0, 53.0), v[0]);
        assert!(dist_to_a < dist_to_b);
    }

    #[test]
    fn unknown_hop_uses_relaxed_fallback() {
        let mut book = ContactBook::default();
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab0001".into(),
            name: "A".into(),
            lat: Some(52.0),
            lon: Some(5.0),
        });
        let hops = vec!["ab".into(), "ff".into()];
        let self_pos = (5.0_f64, 52.0_f64);
        let v = infer_route_lon_lat(&hops, &book, Some(self_pos));
        assert_eq!(v.len(), 2);
        assert!(infer_route_lon_lat_strict(&hops, &book).is_empty());
    }

    #[test]
    fn known_but_no_gps_uses_relaxed_fallback() {
        let mut book = ContactBook::default();
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab0001".into(),
            name: "A".into(),
            lat: Some(52.0),
            lon: Some(5.0),
        });
        book.upsert(ContactRecord {
            hash0: 0xcd,
            pubkey_prefix_hex: "cd0001".into(),
            name: "NoGPS".into(),
            lat: None,
            lon: None,
        });
        let hops = vec!["ab".into(), "cd".into()];
        let self_pos = (5.0_f64, 52.0_f64);
        let v = infer_route_lon_lat(&hops, &book, Some(self_pos));
        assert_eq!(v.len(), 2);
        assert!(infer_route_lon_lat_strict(&hops, &book).is_empty());
    }

    #[test]
    fn reverse_strict_fails_if_only_gps_beyond_30km() {
        let mut book = ContactBook::default();
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab0001".into(),
            name: "Local".into(),
            lat: Some(52.0),
            lon: Some(5.0),
        });
        let hops = vec!["ab".into()];
        let self_far = (5.0_f64, 55.0_f64);
        assert!(infer_route_lon_lat_reverse_from_self_strict(&hops, &book, self_far).is_empty());
    }
}
