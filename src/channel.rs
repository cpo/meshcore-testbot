//! Channel info / group text parsing and outbound channel commands.

use crate::config::reply_location_text;
use crate::protocol::{
    PKT_CHANNEL_INFO, PKT_CHANNEL_MSG, PKT_CHANNEL_MSG_V3, PKT_DEVICE_INFO,
};
use sha2::{Digest, Sha256};

pub const MONITORED_HASHTAGS: &[&str] = &["#bot", "#bots", "#test", "ZeewoldeDenBosch"];
pub const MAX_CHANNEL_TEXT_BYTES: usize = 133;
pub const TRIGGER_TEXTS: &[&str] = &["test", "ontvang", "ping", "echo"];

/// 32-byte companion field is NUL-terminated then padded; only bytes before the first `0` are the name.
pub fn normalize_channel_name(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end])
        .trim()
        .to_string()
}

pub fn channel_matches_monitored(name: &str, secret: &[u8; 16]) -> bool {
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
pub fn channel_message_body_for_trigger(text: &str) -> &str {
    let t = text.trim();
    if let Some((_name, rest)) = t.split_once(": ") {
        rest.trim()
    } else {
        t
    }
}

/// Author prefix from `name: …` group format; `None` if there is no `": "` prefix or the name is empty.
pub fn author_name_from_channel_text(text: &str) -> Option<&str> {
    let t = text.trim();
    let (name, _) = t.split_once(": ")?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Body must match one configured trigger exactly (ASCII case-insensitive): either the whole trimmed text or the part after `name: `. Signed messages strip the 4-byte pubkey prefix in the parser first.
pub fn message_is_exactly_test(text: &str) -> bool {
    let body = channel_message_body_for_trigger(text);
    TRIGGER_TEXTS
        .iter()
        .any(|trigger| body.eq_ignore_ascii_case(trigger))
}

pub fn looks_like_our_reply(text: &str) -> bool {
    text.contains(reply_location_text())
}

#[derive(Debug, Clone)]
pub struct PathInfo {
    pub flood: bool,
    pub hops: Option<u8>,
}

pub fn decode_path_byte(plen: u8) -> PathInfo {
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

pub fn hops_label(path: &PathInfo) -> String {
    if path.flood {
        "flood".to_string()
    } else {
        format!("{} hops", path.hops.unwrap_or(0))
    }
}

pub fn clamp_meshcore_utf8(s: String) -> String {
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

fn hex_prefix_4(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn strip_signed_prefix(text_offset: usize, txt_type: u8, data: &[u8]) -> (Option<String>, String) {
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

pub struct ChannelIncoming {
    pub channel_idx: u8,
    pub path: PathInfo,
    pub text: String,
}

pub fn parse_channel_message(data: &[u8]) -> Option<ChannelIncoming> {
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

pub fn parse_channel_info(data: &[u8]) -> Option<(u8, String, [u8; 16])> {
    if data.first().copied()? != PKT_CHANNEL_INFO || data.len() < 50 {
        return None;
    }
    let idx = data[1];
    let name = normalize_channel_name(&data[2..34]);
    let secret: [u8; 16] = data[34..50].try_into().ok()?;
    Some((idx, name, secret))
}

pub fn parse_device_info_max_channels(data: &[u8]) -> Option<u8> {
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

pub fn get_channel_cmd(idx: u8) -> Vec<u8> {
    vec![0x1f, idx]
}

pub fn send_channel_msg_cmd(channel_idx: u8, text: &str) -> Vec<u8> {
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
