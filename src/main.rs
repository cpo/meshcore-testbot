//! MeshCore companion bot over **USB serial** or **TCP** (same framing as MeshCore `ArduinoSerialInterface`:
//! send `<` + u16 LE len + payload, receive `>` + u16 LE len + payload).
//!
//! - **USB:** set `MESHCORE_SERIAL` to the device path (e.g. `/dev/ttyACM0`). Optional: `MESHCORE_BAUD` (default `115200`).
//! - **TCP:** set `MESHCORE_TCP` to `host:port` (e.g. `192.168.1.5:5000`). If set, TCP is used and `MESHCORE_SERIAL` is ignored.
//!
//! Optional for both: `MESHCORE_POLL_SECS` (default `3`) for periodic `GET_MESSAGE` polling.
//! Set `MESHCORE_LOGALL` (any non-empty value) to log every companion packet **in and out** (hex dump and parsed summary on stderr).
//! Channel message packets (`0x08` / `0x11`) are always printed on receive (same format), even when `MESHCORE_LOGALL` is unset.
//!
//! Replies are sent on the **same channel index** as the incoming message (among monitored `#bot` / `#bots` / `#test` slots).
//! Trigger is the word `Test` (case-insensitive) alone, or after a MeshCore-style `name: ` prefix (e.g. `Alice: Test`); the reply appends that name when present.
//! Optional: `MESHCORE_REPLY_TEXT` sets the location line in replies (default `Den Bosch Noord`); used for echo detection too.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::io::{Read, Write};
use std::sync::mpsc::Sender as StdSender;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::MissedTickBehavior;

const FRAME_SEND_PREFIX: u8 = 0x3c; // '<'
const FRAME_RECV_PREFIX: u8 = 0x3e; // '>'

const APP_START: &[u8] = b"\x01\x03 meshcorebot";
const DEVICE_QUERY: &[u8] = b"\x16\x03";
const GET_MESSAGE: &[u8] = b"\x0a";

const PKT_SELF_INFO: u8 = 0x05;
const PKT_DEVICE_INFO: u8 = 0x0d;
const PKT_CHANNEL_INFO: u8 = 0x12;
const PKT_CHANNEL_MSG: u8 = 0x08;
const PKT_CHANNEL_MSG_V3: u8 = 0x11;
const PKT_MSG_WAITING: u8 = 0x83;
/// `PACKET_LOG_DATA` — RF RX log (`MyMesh::logRxRaw`).
const PKT_LOG_RX_DATA: u8 = 0x88;
const PKT_ERROR: u8 = 0x01;
const PKT_NO_MORE_MSGS: u8 = 0x0a;

const MONITORED_HASHTAGS: &[&str] = &["#bot", "#bots", "#test", "ZeewoldeDenBosch"];
const MAX_CHANNEL_TEXT_BYTES: usize = 133;
const TRIGGER_TEXT: &str = "Test";
const DEFAULT_REPLY_TEXT: &str = "Den Bosch Noord";

static REPLY_TEXT: OnceLock<String> = OnceLock::new();

/// Configured by `MESHCORE_REPLY_TEXT` (non-empty after trim); default [`DEFAULT_REPLY_TEXT`].
fn reply_location_text() -> &'static str {
    REPLY_TEXT
        .get_or_init(|| {
            env::var("MESHCORE_REPLY_TEXT")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_REPLY_TEXT.to_string())
        })
        .as_str()
}

#[derive(Clone, Copy)]
enum PacketDirection {
    Rx,
    Tx,
}

fn meshcore_log_all_enabled() -> bool {
    env::var("MESHCORE_LOGALL")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
}

fn companion_frame_send(payload: &[u8]) -> Vec<u8> {
    log_outgoing_payload(payload);
    let len = payload.len() as u16;
    let mut v = Vec::with_capacity(3 + payload.len());
    v.push(FRAME_SEND_PREFIX);
    v.extend_from_slice(&len.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

#[async_trait]
pub trait MeshTransport: Send {
    async fn send_payload(&self, payload: &[u8]) -> Result<()>;
    async fn read_frames(&mut self) -> Result<Vec<Vec<u8>>>;
    async fn next_frame(&mut self) -> Result<Vec<u8>> {
        loop {
            let frames = self.read_frames().await?;
            if let Some(f) = frames.into_iter().next() {
                return Ok(f);
            }
        }
    }
}

/// 32-byte companion field is NUL-terminated then padded; only bytes before the first `0` are the name.
fn normalize_channel_name(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end])
        .trim()
        .to_string()
}

fn channel_matches_monitored(name: &str, secret: &[u8; 16]) -> bool {
    for tag in MONITORED_HASHTAGS {
        if name == *tag {
            return true;
        }
        let expected: [u8; 16] = Sha256::digest(tag.as_bytes())[..16].try_into().expect("16 bytes");
        if *secret == expected {
            return true;
        }
    }
    false
}

/// MeshCore group text is typically `name: message` (see `payloads.md`). Body used for the trigger is the part after the first `": "`, or the whole string if absent.
fn channel_message_body_for_trigger(text: &str) -> &str {
    let t = text.trim();
    if let Some((_name, rest)) = t.split_once(": ") {
        rest.trim()
    } else {
        t
    }
}

/// Author prefix from `name: …` group format; `None` if there is no `": "` prefix or the name is empty.
fn author_name_from_channel_text(text: &str) -> Option<&str> {
    let t = text.trim();
    let (name, _) = t.split_once(": ")?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Body must be exactly `Test` (ASCII case-insensitive): either the whole trimmed text or the part after `name: `. Signed messages strip the 4-byte pubkey prefix in the parser first.
fn message_is_exactly_test(text: &str) -> bool {
    channel_message_body_for_trigger(text).eq_ignore_ascii_case(TRIGGER_TEXT)
}

fn looks_like_our_reply(text: &str) -> bool {
    text.contains(reply_location_text())
}

fn hops_label(path: &PathInfo) -> String {
    if path.flood {
        "flood".to_string()
    } else {
        format!("{} hops", path.hops.unwrap_or(0))
    }
}

fn clamp_meshcore_utf8(s: String) -> String {
    if s.len() <= MAX_CHANNEL_TEXT_BYTES {
        return s;
    }
    let mut t = String::new();
    for ch in s.chars() {
        if t.len() + ch.len_utf8() > MAX_CHANNEL_TEXT_BYTES.saturating_sub(1) {
            t.push('…');
            break;
        }
        t.push(ch);
    }
    t
}

#[derive(Debug, Clone)]
struct PathInfo {
    flood: bool,
    hops: Option<u8>,
}

fn decode_path_byte(plen: u8) -> PathInfo {
    if plen == 255 {
        PathInfo {
            flood: true,
            hops: None,
        }
    } else {
        PathInfo {
            flood: false,
            hops: Some(plen & 0x3f),
        }
    }
}

fn hex_prefix_4(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn strip_signed_prefix(text_offset: usize, txt_type: u8, data: &[u8]) -> (Option<String>, String) {
    let rest = data.get(text_offset..).unwrap_or(&[]);
    if txt_type == 2 && rest.len() >= 4 {
        let prefix_hex = hex_prefix_4(&rest[..4]);
        let body = String::from_utf8_lossy(&rest[4..])
            .trim_end_matches('\0')
            .to_string();
        (Some(prefix_hex), body)
    } else {
        (
            None,
            String::from_utf8_lossy(rest)
                .trim_end_matches('\0')
                .to_string(),
        )
    }
}

struct ChannelIncoming {
    channel_idx: u8,
    path: PathInfo,
    text: String,
}

fn parse_channel_message(data: &[u8]) -> Option<ChannelIncoming> {
    match data.first().copied()? {
        PKT_CHANNEL_MSG if data.len() >= 9 => {
            let ch = data[1];
            let plen = data[2];
            let txt_type = data[3];
            let (_pk, text) = strip_signed_prefix(8, txt_type, data);
            Some(ChannelIncoming {
                channel_idx: ch,
                path: decode_path_byte(plen),
                text,
            })
        }
        PKT_CHANNEL_MSG_V3 if data.len() >= 12 => {
            let ch = data[4];
            let plen = data[5];
            let txt_type = data[6];
            let (_pk, text) = strip_signed_prefix(11, txt_type, data);
            Some(ChannelIncoming {
                channel_idx: ch,
                path: decode_path_byte(plen),
                text,
            })
        }
        _ => None,
    }
}

fn parse_channel_info(data: &[u8]) -> Option<(u8, String, [u8; 16])> {
    if data.first().copied()? != PKT_CHANNEL_INFO || data.len() < 50 {
        return None;
    }
    let idx = data[1];
    let name = normalize_channel_name(&data[2..34]);
    let secret: [u8; 16] = data[34..50].try_into().ok()?;
    Some((idx, name, secret))
}

fn parse_device_info_max_channels(data: &[u8]) -> Option<u8> {
    if data.first().copied()? != PKT_DEVICE_INFO || data.len() < 4 {
        return None;
    }
    let fw = data[1];
    if fw >= 3 {
        Some(data[3])
    } else {
        Some(8)
    }
}

fn hex_preview_for_log(data: &[u8], max_bytes: usize) -> String {
    let take = data.len().min(max_bytes);
    let s: String = data[..take]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if data.len() > take {
        format!("{s} …")
    } else {
        s
    }
}

/// Companion push `0x88` / `PACKET_LOG_DATA`: byte1 = `(snr * 4)` as i8, byte2 = `rssi` as i8, rest = raw OTA bytes (`MyMesh::logRxRaw`).
fn format_packet_log_rx_data(packet: &[u8]) -> String {
    if packet.len() < 3 {
        return format!(
            "LogRxData (truncated, {} bytes) {}",
            packet.len(),
            hex_preview_for_log(packet, 48)
        );
    }
    let snr_raw = packet[1] as i8;
    let rssi_raw = packet[2] as i8;
    let snr = f64::from(snr_raw) / 4.0;
    let raw = &packet[3..];
    format!(
        "LogRxData snr={snr:.2} (q={snr_raw}) rssi={rssi_raw} raw_len={} raw={}",
        raw.len(),
        hex_preview_for_log(raw, 96)
    )
}

fn format_parsed_packet(packet: &[u8], dir: PacketDirection) -> String {
    let Some(&ty) = packet.first() else {
        return "empty packet".to_string();
    };
    if packet.len() == 1 && ty == PKT_NO_MORE_MSGS {
        return match dir {
            PacketDirection::Rx => "NoMoreMsgs".to_string(),
            PacketDirection::Tx => "GetMessage (poll)".to_string(),
        };
    }
    if matches!(dir, PacketDirection::Tx) && packet.starts_with(APP_START) {
        return format!("AppStart ({})", packet.len());
    }
    if matches!(dir, PacketDirection::Tx) && packet.len() >= 2 && ty == 0x16 && packet[1] == 0x03 {
        return "DeviceQuery".to_string();
    }
    if matches!(dir, PacketDirection::Tx) && packet.len() == 2 && packet[0] == 0x1f {
        return format!("GetChannelInfo idx={}", packet[1]);
    }
    if matches!(dir, PacketDirection::Tx) && packet.len() >= 7 && packet[0] == 0x03 && packet[1] == 0x00 {
        let ch = packet[2];
        let text = String::from_utf8_lossy(&packet[7..])
            .trim_end_matches('\0')
            .to_string();
        return format!("SendChannelMsg ch={ch} text={text:?}");
    }
    match ty {
        PKT_SELF_INFO => format!("SelfInfo ({} bytes) {}", packet.len(), hex_preview_for_log(&packet[1..], 32)),
        PKT_DEVICE_INFO => {
            if let Some(mc) = parse_device_info_max_channels(packet) {
                let fw = packet.get(1).copied().unwrap_or(0);
                format!("DeviceInfo fw={fw} max_channels={mc} ({} bytes)", packet.len())
            } else {
                format!(
                    "DeviceInfo parse incomplete ({} bytes) {}",
                    packet.len(),
                    hex_preview_for_log(packet, 48)
                )
            }
        }
        PKT_CHANNEL_INFO => {
            if let Some((idx, name, secret)) = parse_channel_info(packet) {
                let sec_hex = hex_preview_for_log(&secret, 16);
                format!("ChannelInfo idx={idx} name={name:?} secret={sec_hex}")
            } else {
                format!(
                    "ChannelInfo parse failed ({} bytes) {}",
                    packet.len(),
                    hex_preview_for_log(packet, 48)
                )
            }
        }
        PKT_CHANNEL_MSG | PKT_CHANNEL_MSG_V3 => {
            if let Some(msg) = parse_channel_message(packet) {
                let hops = hops_label(&msg.path);
                format!(
                    "ChannelMsg ch={} path={hops} text={:?}",
                    msg.channel_idx, msg.text
                )
            } else {
                format!(
                    "ChannelMsg parse failed ({} bytes) {}",
                    packet.len(),
                    hex_preview_for_log(packet, 48)
                )
            }
        }
        PKT_MSG_WAITING => "MsgWaiting".to_string(),
        PKT_ERROR => {
            if packet.len() > 1 {
                format!(
                    "Error tail={}",
                    hex_preview_for_log(&packet[1..], 64)
                )
            } else {
                "Error (no payload)".to_string()
            }
        }
        PKT_NO_MORE_MSGS => format!("NoMoreMsgs ({} bytes)", packet.len()),
        PKT_LOG_RX_DATA => format_packet_log_rx_data(packet),
        _ => format!(
            "unknown type=0x{ty:02x} len={} {}",
            packet.len(),
            hex_preview_for_log(packet, 32)
        ),
    }
}

fn is_channel_msg_packet(frame: &[u8]) -> bool {
    matches!(
        frame.first().copied(),
        Some(PKT_CHANNEL_MSG) | Some(PKT_CHANNEL_MSG_V3)
    )
}

fn log_received_packets(frames: &[Vec<u8>]) {
    let log_all = meshcore_log_all_enabled();
    for frame in frames {
        if !log_all && !is_channel_msg_packet(frame) {
            continue;
        }
        let hex: String = frame
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        if log_all {
            eprintln!("rx packet {} bytes: {hex}", frame.len());
        }
        eprintln!("  rx: {}", format_parsed_packet(frame, PacketDirection::Rx));
    }
}

fn log_outgoing_payload(payload: &[u8]) {
    if !meshcore_log_all_enabled() {
        return;
    }
    let hex: String = payload
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("tx packet {} bytes: {hex}", payload.len());
    eprintln!("  tx: {}", format_parsed_packet(payload, PacketDirection::Tx));
}

fn get_channel_cmd(idx: u8) -> Vec<u8> {
    vec![0x1f, idx]
}

fn send_channel_msg_cmd(channel_idx: u8, text: &str) -> Vec<u8> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let mut v = Vec::with_capacity(8 + text.len());
    v.extend_from_slice(&[0x03, 0x00, channel_idx]);
    v.extend_from_slice(&ts.to_le_bytes());
    v.extend_from_slice(text.as_bytes());
    v
}

struct RxFramer {
    header: Vec<u8>,
    expected: usize,
    inframe: Vec<u8>,
}

impl RxFramer {
    fn new() -> Self {
        Self {
            header: Vec::new(),
            expected: 0,
            inframe: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut data = chunk;

        loop {
            if self.header.is_empty() {
                let idx = match data.iter().position(|&b| b == FRAME_RECV_PREFIX) {
                    Some(i) => i,
                    None => return out,
                };
                data = &data[idx..];
                self.header.push(data[0]);
                data = &data[1..];
            }

            while self.header.len() < 3 && !data.is_empty() {
                self.header.push(data[0]);
                data = &data[1..];
            }
            if self.header.len() < 3 {
                return out;
            }

            self.expected = u16::from_le_bytes([self.header[1], self.header[2]]) as usize;
            if self.expected > 300 {
                self.header.clear();
                self.inframe.clear();
                self.expected = 0;
                if !data.is_empty() {
                    continue;
                }
                return out;
            }

            let need = self.expected.saturating_sub(self.inframe.len());
            if data.len() < need {
                self.inframe.extend_from_slice(data);
                return out;
            }
            self.inframe.extend_from_slice(&data[..need]);
            data = &data[need..];

            out.push(std::mem::take(&mut self.inframe));
            self.header.clear();
            self.expected = 0;

            if data.is_empty() {
                return out;
            }
        }
    }
}

pub struct SerialThreadTransport {
    frame_rx: UnboundedReceiver<Vec<u8>>,
    write_tx: StdSender<Vec<u8>>,
}

impl SerialThreadTransport {
    pub fn open(path: &str, baud: u32) -> Result<Self> {
        let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (write_tx, write_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let path_owned = path.to_string();
        std::thread::Builder::new()
            .name("meshcore-serial".into())
            .spawn(move || serial_reader_loop(path_owned, baud, frame_tx, write_rx))
            .with_context(|| format!("spawn serial reader for {path}"))?;
        Ok(Self {
            frame_rx,
            write_tx,
        })
    }

    pub fn poll_sender(&self) -> StdSender<Vec<u8>> {
        self.write_tx.clone()
    }
}

fn serial_reader_loop(
    path: String,
    baud: u32,
    frame_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    write_rx: std::sync::mpsc::Receiver<Vec<u8>>,
) {
    let mut port = match serialport::new(&path, baud)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(100))
        .open()
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("serial open failed ({path}): {e}");
            return;
        }
    };
    let _ = port.write_data_terminal_ready(true);
    let _ = port.write_request_to_send(false);
    eprintln!("serial: {path} @ {baud} baud, 8N1, DTR=on RTS=off");

    let mut framer = RxFramer::new();
    let mut buf = [0u8; 8192];
    loop {
        while let Ok(chunk) = write_rx.try_recv() {
            if port.write_all(&chunk).is_err() {
                return;
            }
            let _ = port.flush();
        }
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                for f in framer.push(&buf[..n]) {
                    if frame_tx.send(f).is_err() {
                        return;
                    }
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("serial read: {e}");
                break;
            }
        }
    }
}

#[async_trait]
impl MeshTransport for SerialThreadTransport {
    async fn send_payload(&self, payload: &[u8]) -> Result<()> {
        let frame = companion_frame_send(payload);
        self.write_tx
            .send(frame)
            .map_err(|_| anyhow!("serial writer thread ended"))?;
        Ok(())
    }

    async fn read_frames(&mut self) -> Result<Vec<Vec<u8>>> {
        let first = self
            .frame_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("serial frame channel closed"))?;
        let mut out = vec![first];
        while let Ok(more) = self.frame_rx.try_recv() {
            out.push(more);
        }
        log_received_packets(&out);
        Ok(out)
    }
}

/// TCP companion: same `<`/`>` framing as serial; reader and writer run as async tasks.
pub struct TcpTransport {
    frame_rx: UnboundedReceiver<Vec<u8>>,
    write_tx: UnboundedSender<Vec<u8>>,
}

impl TcpTransport {
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("failed to connect to {addr}"))?;
        let (mut read_half, mut write_half) = stream.into_split();
        let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        tokio::spawn(async move {
            let mut framer = RxFramer::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = match read_half.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("tcp read: {e}");
                        break;
                    }
                };
                for f in framer.push(&buf[..n]) {
                    if frame_tx.send(f).is_err() {
                        return;
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(frame) = write_rx.recv().await {
                if write_half.write_all(&frame).await.is_err() {
                    break;
                }
                let _ = write_half.flush().await;
            }
        });

        eprintln!("tcp: connected to {addr}");
        Ok(Self { frame_rx, write_tx })
    }

    pub fn poll_sender(&self) -> UnboundedSender<Vec<u8>> {
        self.write_tx.clone()
    }
}

#[async_trait]
impl MeshTransport for TcpTransport {
    async fn send_payload(&self, payload: &[u8]) -> Result<()> {
        let frame = companion_frame_send(payload);
        self.write_tx
            .send(frame)
            .map_err(|_| anyhow!("tcp writer closed"))?;
        Ok(())
    }

    async fn read_frames(&mut self) -> Result<Vec<Vec<u8>>> {
        let first = self
            .frame_rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("tcp frame channel closed"))?;
        let mut out = vec![first];
        while let Ok(more) = self.frame_rx.try_recv() {
            out.push(more);
        }
        log_received_packets(&out);
        Ok(out)
    }
}

fn poll_interval_secs() -> u64 {
    env::var("MESHCORE_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

fn spawn_get_message_poll_std(write_tx: StdSender<Vec<u8>>) {
    let poll_secs = poll_interval_secs();
    eprintln!("GET_MESSAGE poll every {poll_secs}s");
    tokio::spawn(async move {
        let mut int = tokio::time::interval(Duration::from_secs(poll_secs));
        int.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            int.tick().await;
            let frame = companion_frame_send(GET_MESSAGE);
            if write_tx.send(frame).is_err() {
                break;
            }
        }
    });
}

fn spawn_get_message_poll_unbounded(write_tx: UnboundedSender<Vec<u8>>) {
    let poll_secs = poll_interval_secs();
    eprintln!("GET_MESSAGE poll every {poll_secs}s");
    tokio::spawn(async move {
        let mut int = tokio::time::interval(Duration::from_secs(poll_secs));
        int.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            int.tick().await;
            let frame = companion_frame_send(GET_MESSAGE);
            if write_tx.send(frame).is_err() {
                break;
            }
        }
    });
}

async fn handle_incoming_frame<T: MeshTransport + ?Sized>(
    companion: &mut T,
    monitored: &HashSet<u8>,
    frame: &[u8],
) -> Result<()> {
    if frame.is_empty() {
        return Ok(());
    }
    match frame[0] {
        PKT_MSG_WAITING => {
            companion.send_payload(GET_MESSAGE).await?;
        }
        PKT_CHANNEL_MSG | PKT_CHANNEL_MSG_V3 => {
            let Some(msg) = parse_channel_message(frame) else {
                return Ok(());
            };
            if monitored.is_empty() || !monitored.contains(&msg.channel_idx) {
                return Ok(());
            }
            if looks_like_our_reply(&msg.text) {
                return Ok(());
            }
            if !message_is_exactly_test(&msg.text) {
                return Ok(());
            }
            let hops = hops_label(&msg.path);
            let loc = reply_location_text();
            let reply = match author_name_from_channel_text(&msg.text) {
                Some(author) => clamp_meshcore_utf8(format!("@[{author}]: {loc} —> {hops}")),
                None => clamp_meshcore_utf8(format!("{loc} — {hops}")),
            };
            eprintln!("reply on ch {}: {reply}", msg.channel_idx);
            companion
                .send_payload(&send_channel_msg_cmd(msg.channel_idx, &reply))
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn drain_pending_messages<T: MeshTransport + ?Sized>(
    companion: &mut T,
    monitored: &HashSet<u8>,
) -> Result<()> {
    for _ in 0..128 {
        companion.send_payload(GET_MESSAGE).await?;
        let pkt = companion.next_frame().await?;
        match pkt.first().copied() {
            Some(PKT_NO_MORE_MSGS) => return Ok(()),
            _ => handle_incoming_frame(companion, monitored, &pkt).await?,
        }
    }
    eprintln!("warning: drain stopped after 128 rounds");
    Ok(())
}

async fn wait_for_packet<T: MeshTransport + ?Sized>(companion: &mut T, want: u8) -> Result<Vec<u8>> {
    loop {
        let f = companion.next_frame().await?;
        if f.is_empty() {
            continue;
        }
        if f[0] == want {
            return Ok(f);
        }
        if f[0] == PKT_MSG_WAITING {
            companion.send_payload(GET_MESSAGE).await?;
        }
    }
}

async fn bootstrap<T: MeshTransport + ?Sized>(companion: &mut T) -> Result<HashSet<u8>> {
    companion.send_payload(APP_START).await?;
    let _self_info = wait_for_packet(companion, PKT_SELF_INFO).await?;

    companion.send_payload(DEVICE_QUERY).await?;
    let dev = wait_for_packet(companion, PKT_DEVICE_INFO).await?;
    let max_ch = parse_device_info_max_channels(&dev).unwrap_or(8);

    eprintln!(
        "max_channels={max_ch}, looking for {:?} …",
        MONITORED_HASHTAGS
    );

    let mut monitored = HashSet::new();

    for idx in 0..max_ch.min(8) {
        companion.send_payload(&get_channel_cmd(idx)).await?;
        loop {
            let pkt = companion.next_frame().await?;
            match pkt.first().copied() {
                Some(PKT_CHANNEL_INFO) => {
                    if let Some((i, name, secret)) = parse_channel_info(&pkt) {
                        if channel_matches_monitored(&name, &secret) {
                            eprintln!("monitoring channel index {i} ({name:?})");
                            monitored.insert(i);
                        }
                    }
                    break;
                }
                Some(PKT_ERROR) => break,
                Some(PKT_MSG_WAITING) => {
                    companion.send_payload(GET_MESSAGE).await?;
                }
                _ => {}
            }
        }
    }

    if monitored.is_empty() {
        eprintln!(
            "warning: no slot matches {:?}; add those hashtag channels on the radio",
            MONITORED_HASHTAGS
        );
    }

    Ok(monitored)
}

async fn run_bot_inner<T: MeshTransport>(transport: &mut T, transport_label: &str) -> Result<()> {
    eprintln!("{transport_label}");
    let monitored = bootstrap(transport).await?;
    drain_pending_messages(transport, &monitored).await?;
    loop {
        let frames = transport.read_frames().await?;
        for frame in frames {
            handle_incoming_frame(transport, &monitored, &frame).await?;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let tcp_addr = env::var("MESHCORE_TCP")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(addr) = tcp_addr {
        let mut tcp = TcpTransport::connect(&addr).await?;
        spawn_get_message_poll_unbounded(tcp.poll_sender());
        run_bot_inner(
            &mut tcp,
            &format!(
                "TCP {addr} — trigger: exact message \"{TRIGGER_TEXT}\""
            ),
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
        spawn_get_message_poll_std(serial.poll_sender());
        run_bot_inner(
            &mut serial,
            &format!(
                "USB serial {path} @ {baud} baud — trigger: exact message \"{TRIGGER_TEXT}\""
            ),
        )
        .await
    }
}
