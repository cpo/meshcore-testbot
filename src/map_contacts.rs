//! Contacten uit de publieke MeshCore-kaart-API (`/api/v1/nodes`).

use crate::contact_book::ContactRecord;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::time::Duration;

const DEFAULT_MAP_NODES_URL: &str = "https://map.meshcore.io/api/v1/nodes?binary=0&short=1";

fn map_nodes_url() -> String {
    env::var("MESHCORE_MAP_NODES_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAP_NODES_URL.to_string())
}

fn map_fetch_timeout_secs() -> u64 {
    env::var("MESHCORE_MAP_FETCH_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1000)
}

#[derive(Debug, Deserialize)]
struct MapNodeJson {
    public_key: String,
    #[serde(default)]
    adv_name: String,
    adv_lat: Option<f64>,
    adv_lon: Option<f64>,
}

fn contact_from_map_node(n: MapNodeJson) -> Option<ContactRecord> {
    let pk_hex = n.public_key.trim();
    if pk_hex.len() != 64 {
        return None;
    }
    let mut pk = [0u8; 32];
    hex::decode_to_slice(pk_hex, &mut pk).ok()?;
    let pk_prefix_len = crate::contact_book::PUBKEY_PREFIX_BYTES.min(32);
    let pubkey_prefix_hex: String = pk[..pk_prefix_len]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut lat = n.adv_lat;
    let mut lon = n.adv_lon;
    if let (Some(la), Some(lo)) = (lat, lon) {
        if !(-90.0..=90.0).contains(&la) || !(-180.0..=180.0).contains(&lo) {
            lat = None;
            lon = None;
        }
    } else {
        lat = None;
        lon = None;
    }
    Some(ContactRecord {
        hash0: pk[0],
        pubkey_prefix_hex,
        name: n.adv_name,
        lat,
        lon,
    })
}

/// Haal alle kaart-stations op en zet ze om naar [`ContactRecord`].
pub async fn fetch_map_contacts() -> Result<Vec<ContactRecord>> {
    let url = map_nodes_url();
    let timeout = Duration::from_secs(map_fetch_timeout_secs());
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("reqwest client")?;
    eprintln!("visor: fetching map nodes from {url} …");
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url} status"))?;
    let nodes: Vec<MapNodeJson> = resp
        .json()
        .await
        .with_context(|| format!("parse JSON from {url}"))?;
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        if let Some(c) = contact_from_map_node(n) {
            out.push(c);
        }
    }
    if out.is_empty() {
        eprintln!("visor: warning: map API returned no usable nodes (invalid public_key rows?)");
    }
    Ok(out)
}
