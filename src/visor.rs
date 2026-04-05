//! HTTP + WebSocket voor route-visualisatie; statische Vue-build uit `frontend/dist`.

use crate::contact_book::{parse_contact_packet, ContactBook, ContactRecord};
use crate::geo_path::infer_route_lon_lat;
use crate::mesh_raw::parse_mesh_path_hops_hex;
use crate::protocol::{
    PKT_CONTACT, PKT_CONTACT_END, PKT_CONTACT_START, PKT_LOG_RX_DATA, PKT_SELF_INFO, PUSH_NEW_ADVERT,
};
use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct VisorHub {
    tx: broadcast::Sender<String>,
    contacts: Arc<Mutex<ContactBook>>,
    self_pos: Arc<Mutex<(Option<f64>, Option<f64>)>>,
    /// Totaal na dedup na de laatste kaart-API-sync (voor de UI: `retrieved / total`).
    reported_contact_total: Arc<Mutex<Option<u32>>>,
}

impl VisorHub {
    pub fn new() -> Self {
        // Ruim genoeg: bij massa-updates (RF-log) niet op Lagged uitkomen.
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            contacts: Arc::new(Mutex::new(ContactBook::default())),
            self_pos: Arc::new(Mutex::new((None, None))),
            reported_contact_total: Arc::new(Mutex::new(None)),
        }
    }

    /// Stuurt JSON naar alle WebSocket-abonnees; grote payloads alleen als lengte loggen.
    fn broadcast_ws_json(&self, json: String) {
        if json.len() > 4096 {
            eprintln!("visor ws out: {} bytes (log ingekort)", json.len());
        } else {
            eprintln!("visor ws out: {json}");
        }
        let _ = self.tx.send(json);
    }

    /// Stuurt de huidige contactenlijst naar alle WebSocket-clients (o.a. na sync-einde).
    pub fn broadcast_contacts_snapshot(&self) {
        if let Some(json) = self.contacts_snapshot_json() {
            self.broadcast_ws_json(json);
        }
    }

    /// JSON voor `type: contacts` (init + updates): geen volledige contactenlijst — alleen indexgrootte + eigen positie.
    pub fn contacts_snapshot_json(&self) -> Option<String> {
        let self_pos = self.self_pos.lock().ok().and_then(|p| {
            let (lo, la) = *p;
            match (lo, la) {
                (Some(lon), Some(lat)) => Some([lat, lon]),
                _ => None,
            }
        });
        let reported_total = self
            .reported_contact_total
            .lock()
            .ok()
            .and_then(|g| *g);
        let msg = ContactsWsMsg {
            msg_type: "contacts",
            self_pos,
            reported_total,
        };
        serde_json::to_string(&msg).ok()
    }

    /// Vervangt het boek door een volledige kaart-sync; `reported_total` = aantal na dedup (voor de UI).
    pub fn bulk_replace_contacts(&self, records: Vec<ContactRecord>) {
        let n = if let Ok(mut g) = self.contacts.lock() {
            g.clear();
            for c in records {
                g.upsert(c);
            }
            g.all_contacts_deduped().len() as u32
        } else {
            return;
        };
        if let Ok(mut t) = self.reported_contact_total.lock() {
            *t = Some(n);
        }
        self.broadcast_contacts_snapshot();
    }

    /// Aantal unieke contacten (pubkey-prefix) in het boek.
    pub fn contact_count_deduped(&self) -> usize {
        self.contacts
            .lock()
            .ok()
            .map(|g| g.all_contacts_deduped().len())
            .unwrap_or(0)
    }

    /// Verwerk elk companion-frame: contacten, self-locatie, en `0x88` RF-log voor pad-inferentie.
    pub fn process_frame(&self, frame: &[u8]) {
        let Some(&ty) = frame.first() else {
            return;
        };
        match ty {
            // Contactlijst komt van map.meshcore.io (zie `sync_contacts`); radio-GET_CONTACTS-stream negeren.
            PKT_CONTACT_START | PKT_CONTACT_END | PKT_CONTACT => {}
            PUSH_NEW_ADVERT => {
                if let Some(c) = parse_contact_packet(frame) {
                    if let Ok(mut g) = self.contacts.lock() {
                        g.upsert(c);
                    }
                    self.broadcast_contacts_snapshot();
                }
            }
            PKT_SELF_INFO => {
                if let Some((lo, la)) = parse_self_info_lon_lat(frame) {
                    if let Ok(mut p) = self.self_pos.lock() {
                        *p = (Some(lo), Some(la));
                    }
                    self.broadcast_contacts_snapshot();
                }
            }
            PKT_LOG_RX_DATA if frame.len() >= 4 => {
                let snr_raw = frame[1] as i8;
                let rssi_raw = frame[2] as i8;
                let snr = f64::from(snr_raw) / 4.0;
                let raw = &frame[3..];
                let Some(hops) = parse_mesh_path_hops_hex(raw) else {
                    return;
                };
                let book = match self.contacts.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let self_pos = self
                    .self_pos
                    .lock()
                    .ok()
                    .and_then(|p| match *p {
                        (Some(lon), Some(lat)) => Some((lon, lat)),
                        _ => None,
                    });
                let coords = infer_route_lon_lat(&hops, &book, self_pos);
                if coords.is_empty() {
                    return;
                }
                let coords_ll: Vec<[f64; 2]> =
                    coords.iter().map(|&(lon, lat)| [lat, lon]).collect();
                let hop_steps: Vec<RouteHopStep> = hops
                    .iter()
                    .enumerate()
                    .map(|(i, hop)| {
                        let c = coords.get(i).and_then(|&(lon, lat)| {
                            book.contact_for_inferred_point(hop, (lon, lat))
                        });
                        let (name, pubkey_prefix_hex, lat, lon) = match &c {
                            Some(r) => (
                                Some(r.name.trim().to_string()).filter(|s| !s.is_empty()),
                                Some(r.pubkey_prefix_hex.clone()),
                                r.lat,
                                r.lon,
                            ),
                            None => (None, None, None, None),
                        };
                        RouteHopStep {
                            hop_hex: hop.clone(),
                            name,
                            pubkey_prefix_hex,
                            lat,
                            lon,
                        }
                    })
                    .collect();
                let id = format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                );
                let msg = RouteWsMsg {
                    msg_type: "route",
                    id,
                    hops_hex: hops,
                    coords: coords_ll,
                    hop_steps,
                    snr,
                    rssi: rssi_raw,
                    inferred: true,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    self.broadcast_ws_json(json);
                }
            }
            _ => {}
        }
    }
}

/// `PACKET_SELF_INFO` volgens MeshCore `companion_protocol.md`: pubkey bytes 4–35, daarna adv-lat/lon.
fn parse_self_info_lon_lat(data: &[u8]) -> Option<(f64, f64)> {
    if data.first().copied()? != PKT_SELF_INFO || data.len() < 44 {
        return None;
    }
    let lat_i = i32::from_le_bytes(data[36..40].try_into().ok()?);
    let lon_i = i32::from_le_bytes(data[40..44].try_into().ok()?);
    if lat_i == 0 && lon_i == 0 {
        return None;
    }
    let lat = lat_i as f64 / 1_000_000.0;
    let lon = lon_i as f64 / 1_000_000.0;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lon, lat))
}

#[derive(Serialize)]
struct ContactsWsMsg {
    #[serde(rename = "type")]
    msg_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    self_pos: Option<[f64; 2]>,
    /// Aantal stations in de server-side index (kaart-API); geen lijst in dit bericht.
    #[serde(skip_serializing_if = "Option::is_none")]
    reported_total: Option<u32>,
}

#[derive(Serialize)]
struct RouteHopStep {
    hop_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pubkey_prefix_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lon: Option<f64>,
}

#[derive(Serialize)]
struct RouteWsMsg {
    #[serde(rename = "type")]
    msg_type: &'static str,
    id: String,
    hops_hex: Vec<String>,
    /// `[lat, lon]` per punt voor Leaflet.
    coords: Vec<[f64; 2]>,
    /// Contactinfo per hop (zelfde volgorde als `hops_hex` / `coords`).
    hop_steps: Vec<RouteHopStep>,
    snr: f64,
    rssi: i8,
    inferred: bool,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<VisorHub>,
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state.hub))
}

async fn handle_socket(mut socket: WebSocket, hub: Arc<VisorHub>) {
    if let Some(json) = hub.contacts_snapshot_json() {
        if json.len() > 4096 {
            eprintln!("visor ws out: contacts init {} bytes", json.len());
        } else {
            eprintln!("visor ws out: {json}");
        }
        if socket.send(Message::Text(json)).await.is_err() {
            return;
        }
    }
    let mut rx = hub.tx.subscribe();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(p))) => {
                        eprintln!(
                            "visor ws out: pong frame (ping payload {} bytes)",
                            p.len()
                        );
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }
    }
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub async fn run_server(hub: Arc<VisorHub>, port: u16) -> Result<()> {
    let static_dir = std::env::var("MESHCORE_FRONTEND_DIST")
        .unwrap_or_else(|_| "frontend/dist".to_string());
    let index_path = Path::new(&static_dir).join("index.html");
    let state = AppState { hub };

    let serve = ServeDir::new(&static_dir).not_found_service(ServeFile::new(index_path));

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .fallback_service(serve)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    eprintln!("visor: http://127.0.0.1:{port}/  WebSocket ws://127.0.0.1:{port}/ws");
    axum::serve(listener, app).await?;
    Ok(())
}
