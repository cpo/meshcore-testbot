//! MeshCore companion bot over **USB serial** or **TCP** (same framing as MeshCore `ArduinoSerialInterface`:
//! send `<` + u16 LE len + payload, receive `>` + u16 LE len + payload).
//!
//! - **USB:** set `MESHCORE_SERIAL` to the device path (e.g. `/dev/ttyACM0`). Optional: `MESHCORE_BAUD` (default `115200`).
//! - **TCP:** set `MESHCORE_TCP` to `host:port` (e.g. `192.168.1.5:5000`). If set, TCP is used and `MESHCORE_SERIAL` is ignored.
//!
//! Optional for both: `MESHCORE_POLL_SECS` (default `3`) for periodic `GET_MESSAGE` polling.
//! Set `MESHCORE_LOGALL` (any non-empty value) to log every companion packet **in and out** (hex dump and parsed summary on stderr).
//! Channel message packets (`0x08` / `0x11`) are always printed on receive (same format), even when `MESHCORE_LOGALL` is unset.
//! Set `MESHCORE_BOT_ENABLED` (non-empty) to send trigger replies on channels; if unset, the visor still runs but auto-replies are off.
//!
//! **Route visualizer:** `MESHCORE_VISOR_PORT` (default `3847`) starts an HTTP server + WebSocket (`/ws`) and serves the Vue
//! build from `frontend/dist` (override with `MESHCORE_FRONTEND_DIST`). Contacten worden opgehaald van de MeshCore-kaart-API
//! (`MESHCORE_MAP_NODES_URL`, default `https://map.meshcore.io/api/v1/nodes?binary=0&short=1`) na bootstrap en periodiek
//! (`MESHCORE_CONTACT_SYNC_SECS`, default `300` = 5 min); paden worden afgeleid uit `0x88` RF-log + contactboek en naar
//! clients gepusht.
//!
//! Replies are sent on the **same channel index** as the incoming message (among monitored `#bot` / `#bots` / `#test` slots).
//! Trigger is the word `Test` (case-insensitive) alone, or after a MeshCore-style `name: ` prefix (e.g. `Alice: Test`); the reply appends that name when present.
//! Optional: `MESHCORE_REPLY_TEXT` sets the location line in replies (default `Den Bosch Noord`); used for echo detection too.

mod channel;
mod config;
mod contact_book;
mod framing;
mod geo_path;
mod map_contacts;
mod mesh_raw;
mod packet_log;
mod protocol;
mod runtime;
mod transport;
mod visor;

use anyhow::{Context, Result};
use channel::TRIGGER_TEXTS;
use std::env;
use std::sync::Arc;
use transport::{SerialThreadTransport, TcpTransport};

#[tokio::main]
async fn main() -> Result<()> {
    let hub = Arc::new(visor::VisorHub::new());
    let visor_port: u16 = env::var("MESHCORE_VISOR_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3847);
    let hub_server = hub.clone();
    tokio::spawn(async move {
        if let Err(e) = visor::run_server(hub_server, visor_port).await {
            eprintln!("visor server: {e}");
        }
    });

    let tcp_addr = env::var("MESHCORE_TCP")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(addr) = tcp_addr {
        let mut tcp = TcpTransport::connect(&addr).await?;
        runtime::spawn_get_message_poll_unbounded(tcp.poll_sender());
        runtime::run_bot_inner(
            &mut tcp,
            &format!(
                "TCP {addr} — trigger: exact message in {:?}",
                TRIGGER_TEXTS
            ),
            &hub,
        )
        .await
    } else {
        let path = env::var("MESHCORE_SERIAL").context(
            "Set MESHCORE_SERIAL to your USB serial device (e.g. /dev/ttyACM0), or MESHCORE_TCP to host:port",
        )?;
        let baud: u32 = env::var("MESHCORE_BAUD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(115200);

        let mut serial = SerialThreadTransport::open(&path, baud)?;
        runtime::spawn_get_message_poll_std(serial.poll_sender());
        runtime::run_bot_inner(
            &mut serial,
            &format!(
                "USB serial {path} @ {baud} baud — trigger: exact message in {:?}",
                TRIGGER_TEXTS
            ),
            &hub,
        )
        .await
    }
}
