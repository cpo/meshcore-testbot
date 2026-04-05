//! Hex / human-readable companion packet logging.

use crate::channel::{
    hops_label, parse_channel_info, parse_channel_message, parse_device_info_max_channels,
};
use crate::config::meshcore_log_all_enabled;
use crate::framing::encode_companion_frame;
use crate::protocol::{
    APP_START, PKT_CHANNEL_INFO, PKT_CHANNEL_MSG, PKT_CHANNEL_MSG_V3, PKT_DEVICE_INFO,
    PKT_ERROR, PKT_LOG_RX_DATA, PKT_MSG_WAITING, PKT_NO_MORE_MSGS, PKT_SELF_INFO,
};

#[derive(Clone, Copy)]
pub enum PacketDirection {
    Rx,
    Tx,
}

pub fn hex_preview_for_log(data: &[u8], max_bytes: usize) -> String {
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
pub fn format_packet_log_rx_data(packet: &[u8]) -> String {
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

pub fn format_parsed_packet(packet: &[u8], dir: PacketDirection) -> String {
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
        PKT_SELF_INFO => format!(
            "SelfInfo ({} bytes) {}",
            packet.len(),
            hex_preview_for_log(&packet[1..], 32)
        ),
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

pub fn is_channel_msg_packet(frame: &[u8]) -> bool {
    matches!(
        frame.first().copied(),
        Some(PKT_CHANNEL_MSG) | Some(PKT_CHANNEL_MSG_V3)
    )
}

pub fn log_received_packets(frames: &[Vec<u8>]) {
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

pub fn log_outgoing_payload(payload: &[u8]) {
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

/// Log (if enabled) then build the `<` + len + payload wire frame.
pub fn companion_frame_send(payload: &[u8]) -> Vec<u8> {
    log_outgoing_payload(payload);
    encode_companion_frame(payload)
}
