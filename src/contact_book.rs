//! Contacten uit companion `PACKET_CONTACT` (0x03) voor pad-reconstructie.
//!
//! Layout volgens `examples/companion_radio/MyMesh.cpp` (`writeContactRespFrame` /
//! `updateContactFromFrame`): pubkey, type, flags, out_path_len, `out_path`\[64\], naam\[32\],
//! timestamp, gps_lat, gps_lon.

use serde::Serialize;
use std::collections::HashMap;

const PUB_KEY_SIZE: usize = 32;
/// Eerste zoveel bytes van de pubkey als hex-prefix voor pad-matching (langere hops = minder collisions).
pub const PUBKEY_PREFIX_BYTES: usize = 8;
const MAX_PATH_SIZE: usize = 64;
const NAME_FIELD_LEN: usize = 32;
/// Byte-index eerste byte van `gps_lat` (inclusief leading 0x03).
const OFF_CONTACT_GPS_LAT: usize = 1 + PUB_KEY_SIZE + 3 + MAX_PATH_SIZE + NAME_FIELD_LEN + 4;

#[derive(Clone, Debug, Serialize)]
pub struct ContactRecord {
    /// Eerste byte van de publieke sleutel (node-hash prefix).
    pub hash0: u8,
    pub pubkey_prefix_hex: String,
    pub name: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lon1, lat1) = (a.0.to_radians(), a.1.to_radians());
    let (lon2, lat2) = (b.0.to_radians(), b.1.to_radians());
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * h.sqrt().asin() * 6371.0
}

fn normalize_field(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).trim().to_string()
}

/// `PACKET_CONTACT` (0x03) en `PUSH_CODE_NEW_ADVERT` (0x8A) — zelfde layout als
/// `MyMesh::writeContactRespFrame` (ook voor nieuwe advert-updates).
pub fn parse_contact_packet(data: &[u8]) -> Option<ContactRecord> {
    let code = data.first().copied()?;
    if !matches!(code, 0x03 | 0x8a) || data.len() < 1 + PUB_KEY_SIZE + 3 + MAX_PATH_SIZE {
        return None;
    }
    let pk = &data[1..1 + PUB_KEY_SIZE];
    let name_start = 1 + PUB_KEY_SIZE + 3 + MAX_PATH_SIZE;
    let name = normalize_field(&data[name_start..(name_start + NAME_FIELD_LEN).min(data.len())]);
    let mut lat = None;
    let mut lon = None;
    if data.len() >= OFF_CONTACT_GPS_LAT + 8 {
        let lat_i = i32::from_le_bytes(data[OFF_CONTACT_GPS_LAT..OFF_CONTACT_GPS_LAT + 4].try_into().ok()?);
        let lon_i = i32::from_le_bytes(
            data[OFF_CONTACT_GPS_LAT + 4..OFF_CONTACT_GPS_LAT + 8]
                .try_into()
                .ok()?,
        );
        if lat_i != 0 || lon_i != 0 {
            let la = lat_i as f64 / 1_000_000.0;
            let lo = lon_i as f64 / 1_000_000.0;
            if (-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo) {
                lat = Some(la);
                lon = Some(lo);
            }
        }
    }
    let n = PUBKEY_PREFIX_BYTES.min(PUB_KEY_SIZE);
    let pubkey_prefix_hex: String = pk[..n].iter().map(|b| format!("{b:02x}")).collect();
    Some(ContactRecord {
        hash0: pk[0],
        pubkey_prefix_hex,
        name,
        lat,
        lon,
    })
}

#[derive(Default)]
pub struct ContactBook {
    /// Meerdere contacten kunnen dezelfde eerste hash-byte hebben.
    by_hash0: HashMap<u8, Vec<ContactRecord>>,
}

impl ContactBook {
    pub fn clear(&mut self) {
        self.by_hash0.clear();
    }

    pub fn upsert(&mut self, c: ContactRecord) {
        self.by_hash0.entry(c.hash0).or_default().push(c);
    }

    pub fn candidates(&self, hash0: u8) -> Vec<&ContactRecord> {
        self.by_hash0.get(&hash0).map(|v| v.iter().collect()).unwrap_or_default()
    }

    /// Contacten waarvan `pubkey_prefix_hex` begint met de hex van deze hop (meest specifiek).
    /// Geen match: val terug op alleen het eerste byte (hash0), zoals voorheen.
    pub fn contacts_for_hop_prefix(&self, hop_hex: &str) -> Vec<&ContactRecord> {
        let h = hop_hex.trim().to_lowercase();
        if h.len() < 2 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return vec![];
        }
        let h = if h.len() % 2 == 1 {
            format!("0{h}")
        } else {
            h
        };
        let mut out: Vec<&ContactRecord> = Vec::new();
        for v in self.by_hash0.values() {
            for c in v {
                if c.pubkey_prefix_hex.to_lowercase().starts_with(&h) {
                    out.push(c);
                }
            }
        }
        if !out.is_empty() {
            return out;
        }
        u8::from_str_radix(&h[..2], 16)
            .map(|b0| self.candidates(b0))
            .unwrap_or_default()
    }

    /// Eén contact voor weergave bij deze hop (GPS-voorkeur bij meerdere matches).
    pub fn resolve_contact_for_hop(&self, hop_hex: &str) -> Option<ContactRecord> {
        let cands = self.contacts_for_hop_prefix(hop_hex);
        if cands.is_empty() {
            return None;
        }
        if cands.len() == 1 {
            return Some(cands[0].clone());
        }
        cands
            .iter()
            .find(|c| c.lat.is_some() && c.lon.is_some())
            .map(|c| (*c).clone())
            .or_else(|| cands.first().map(|c| (*c).clone()))
    }

    /// Label voor een geïnferneerd padpunt: kies het contact met dezelfde hop-prefix waarvan de GPS
    /// het **dichtst bij** `(lon, lat)` ligt — dat volgt de padinferentie (dichtstbij vorige knoop),
    /// niet een willekeurige `resolve_contact_for_hop`-tiebreak.
    pub fn contact_for_inferred_point(
        &self,
        hop_hex: &str,
        lon_lat: (f64, f64),
    ) -> Option<ContactRecord> {
        let with_gps: Vec<&ContactRecord> = self
            .contacts_for_hop_prefix(hop_hex)
            .into_iter()
            .filter(|c| c.lat.is_some() && c.lon.is_some())
            .collect();
        if with_gps.is_empty() {
            return self.resolve_contact_for_hop(hop_hex);
        }
        let best = with_gps.iter().min_by(|a, b| {
            let da = haversine_km(lon_lat, (a.lon.unwrap(), a.lat.unwrap()));
            let db = haversine_km(lon_lat, (b.lon.unwrap(), b.lat.unwrap()));
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        best.map(|c| (*c).clone())
    }

    /// Platte lijst voor de visor; laatste contact wint bij dezelfde `pubkey_prefix_hex`.
    pub fn all_contacts_deduped(&self) -> Vec<ContactRecord> {
        let mut m: HashMap<String, ContactRecord> = HashMap::new();
        for v in self.by_hash0.values() {
            for c in v {
                m.insert(c.pubkey_prefix_hex.clone(), c.clone());
            }
        }
        m.into_values().collect()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_push_new_advert() {
        let mut data = vec![0u8; 148];
        data[0] = 0x8a;
        data[1..33].copy_from_slice(&[0xab; 32]);
        let mut name = [0u8; 32];
        name[..12].copy_from_slice(b"TestStation\0");
        data[100..132].copy_from_slice(&name);
        let lat_i = (52000000i32).to_le_bytes();
        let lon_i = (5000000i32).to_le_bytes();
        let off = OFF_CONTACT_GPS_LAT;
        data[off..off + 4].copy_from_slice(&lat_i);
        data[off + 4..off + 8].copy_from_slice(&lon_i);
        let c = parse_contact_packet(&data).expect("0x8A contact");
        assert_eq!(c.hash0, 0xab);
        assert!(c.name.contains("TestStation"));
        assert!(c.lat.is_some());
    }

    #[test]
    fn contact_for_inferred_point_matches_nearest_station_gps() {
        let mut book = ContactBook::default();
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab11111111111111".into(),
            name: "Far".into(),
            lat: Some(53.0),
            lon: Some(5.0),
        });
        book.upsert(ContactRecord {
            hash0: 0xab,
            pubkey_prefix_hex: "ab22222222222222".into(),
            name: "Near".into(),
            lat: Some(52.0),
            lon: Some(5.0),
        });
        let c = book
            .contact_for_inferred_point("ab", (5.0, 52.01))
            .expect("nearest");
        assert_eq!(c.name, "Near");
    }
}
