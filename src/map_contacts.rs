//! Contacten uit de publieke MeshCore-kaart-API (`/api/v1/nodes`, o.a. `map.meshcore.io`).
//!
//! Rijen waarvan `updated_date` ouder is dan 14 dagen worden genegeerd (geen kaartindex).
//! Ontbreekt het veld (oude cache-export), dan blijft de node staan.
//!
//! De ruwe JSON-response kan in een lokaal bestand worden gezet; bij opstart wordt dat
//! gelezen i.p.v. opnieuw de API aan te roepen ([`MapLoadMode::PreferCache`]).
//! Periodieke sync gebruikt het netwerk en ververst het bestand ([`MapLoadMode::NetworkRefresh`]).

use crate::contact_book::ContactRecord;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

const DEFAULT_MAP_NODES_URL: &str = "https://map.meshcore.io/api/v1/nodes?binary=0&short=1";
const DEFAULT_CACHE_FILENAME: &str = "meshcore_map_nodes_cache.json";
/// Nodes met `updated_date` ouder dan dit worden niet op de kaart/contactindex gezet.
const MAP_NODE_MAX_AGE: chrono::Duration = chrono::Duration::days(14);

fn map_nodes_url() -> String {
    env::var("MESHCORE_MAP_NODES_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAP_NODES_URL.to_string())
}

fn map_cache_file_path() -> PathBuf {
    env::var("MESHCORE_MAP_CACHE_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CACHE_FILENAME))
}

fn map_cache_disabled() -> bool {
    env::var("MESHCORE_MAP_CACHE_DISABLE")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
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
    /// RFC3339 van de kaart-API; ontbreekt in oude cache → node blijft meetellen.
    #[serde(default)]
    updated_date: Option<DateTime<Utc>>,
}

/// Hoe kaartdata wordt geladen.
#[derive(Clone, Copy, Debug)]
pub enum MapLoadMode {
    /// Cachebestand gebruiken als het bestaat en geldig is; anders HTTP en cache wegschrijven.
    PreferCache,
    /// Altijd HTTP; response naar cache schrijven. Bij netwerkfout: terugvallen op cache indien aanwezig.
    NetworkRefresh,
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

fn records_from_nodes(nodes: Vec<MapNodeJson>) -> Vec<ContactRecord> {
    let cutoff = Utc::now() - MAP_NODE_MAX_AGE;
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        if let Some(ts) = n.updated_date {
            if ts < cutoff {
                continue;
            }
        }
        if let Some(c) = contact_from_map_node(n) {
            out.push(c);
        }
    }
    out
}

async fn read_nodes_from_cache_path(path: &Path) -> Result<Vec<MapNodeJson>> {
    let raw = fs::read_to_string(path)
        .await
        .with_context(|| format!("read map cache {}", path.display()))?;
    let nodes: Vec<MapNodeJson> = serde_json::from_str(&raw)
        .with_context(|| format!("parse map cache JSON {}", path.display()))?;
    Ok(nodes)
}

async fn try_load_cache(path: &Path) -> Option<Vec<MapNodeJson>> {
    if !path.is_file() {
        return None;
    }
    match read_nodes_from_cache_path(path).await {
        Ok(n) if !n.is_empty() => Some(n),
        Ok(_) => {
            eprintln!(
                "visor: map cache {} is empty array; ignoring",
                path.display()
            );
            None
        }
        Err(e) => {
            eprintln!("visor: map cache {} invalid: {e:#}", path.display());
            None
        }
    }
}

async fn write_cache_raw(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create cache dir {}", parent.display()))?;
        }
    }
    fs::write(path, body.as_bytes())
        .await
        .with_context(|| format!("write map cache {}", path.display()))?;
    Ok(())
}

async fn fetch_from_network_and_cache(path: &Path) -> Result<Vec<MapNodeJson>> {
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
    let body = resp
        .text()
        .await
        .with_context(|| format!("read body from {url}"))?;

    if !map_cache_disabled() {
        if let Err(e) = write_cache_raw(path, &body).await {
            eprintln!("visor: warning: could not write map cache {}: {e:#}", path.display());
        } else {
            eprintln!("visor: saved map API response to {}", path.display());
        }
    }

    let nodes: Vec<MapNodeJson> = serde_json::from_str(&body)
        .with_context(|| format!("parse JSON from {url}"))?;
    Ok(nodes)
}

/// Haal kaart-stations op als [`ContactRecord`], afhankelijk van [`MapLoadMode`] en cache-bestand.
pub async fn fetch_map_contacts(mode: MapLoadMode) -> Result<Vec<ContactRecord>> {
    let path = map_cache_file_path();

    if map_cache_disabled() {
        let nodes = fetch_from_network_and_cache(&path).await?;
        let out = records_from_nodes(nodes);
        if out.is_empty() {
            eprintln!("visor: warning: map API returned no usable nodes (invalid public_key rows?)");
        }
        return Ok(out);
    }

    match mode {
        MapLoadMode::PreferCache => {
            if let Some(nodes) = try_load_cache(&path).await {
                eprintln!(
                    "visor: map nodes from cache file {} ({} rows)",
                    path.display(),
                    nodes.len()
                );
                let out = records_from_nodes(nodes);
                if out.is_empty() {
                    eprintln!("visor: warning: cache produced no usable contacts; fetching from network …");
                    let nodes = fetch_from_network_and_cache(&path).await?;
                    let out = records_from_nodes(nodes);
                    if out.is_empty() {
                        eprintln!(
                            "visor: warning: map API returned no usable nodes (invalid public_key rows?)"
                        );
                    }
                    return Ok(out);
                }
                return Ok(out);
            }
            let nodes = fetch_from_network_and_cache(&path).await?;
            let out = records_from_nodes(nodes);
            if out.is_empty() {
                eprintln!("visor: warning: map API returned no usable nodes (invalid public_key rows?)");
            }
            Ok(out)
        }
        MapLoadMode::NetworkRefresh => match fetch_from_network_and_cache(&path).await {
            Ok(nodes) => {
                let out = records_from_nodes(nodes);
                if out.is_empty() {
                    eprintln!(
                        "visor: warning: map API returned no usable nodes (invalid public_key rows?)"
                    );
                }
                Ok(out)
            }
            Err(e) => {
                eprintln!("visor: warning: map fetch failed ({e:#}); trying cache file …");
                let nodes = read_nodes_from_cache_path(&path).await?;
                let out = records_from_nodes(nodes);
                if out.is_empty() {
                    anyhow::bail!("map fetch failed and cache unusable or empty");
                }
                eprintln!(
                    "visor: using stale map cache {} ({} contacts)",
                    path.display(),
                    out.len()
                );
                Ok(out)
            }
        },
    }
}
