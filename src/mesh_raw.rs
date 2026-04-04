//! Ruwe RF-bytes uit `0x88` log (`logRxRaw`) — pad volgens MeshCore `packet_format.md`.

/// `PAYLOAD_TYPE_GRP_TXT`
pub const PAYLOAD_GRP_TXT: u8 = 0x05;

/// Parseert pad-hashketen: per hop `hash_size` bytes (1–4) als **doorlopende** hex (geen spaties),
/// zodat ze te matchen zijn met het begin van `ContactRecord::pubkey_prefix_hex`.
pub fn parse_mesh_path_hops_hex(raw: &[u8]) -> Option<Vec<String>> {
    if raw.is_empty() {
        return None;
    }
    let header = raw[0];
    let payload_type = (header >> 2) & 0x0f;
    if payload_type != PAYLOAD_GRP_TXT {
        return None;
    }
    let route = header & 0x03;
    let mut offset = 1usize;
    if route == 0x00 || route == 0x03 {
        offset = offset.checked_add(4)?;
        if raw.len() < offset {
            return None;
        }
    }
    if raw.len() <= offset {
        return None;
    }
    let path_len_byte = raw[offset];
    offset += 1;
    let hop_count = (path_len_byte & 0x3f) as usize;
    let hash_size = ((path_len_byte >> 6) & 0x03) as usize + 1;
    let path_byte_len = hop_count.checked_mul(hash_size)?;
    if raw.len() < offset + path_byte_len || hop_count == 0 {
        return None;
    }
    let path_bytes = &raw[offset..offset + path_byte_len];
    let parts: Vec<String> = path_bytes
        .chunks(hash_size)
        .map(|chunk| chunk.iter().map(|b| format!("{:02x}", b)).collect())
        .collect();
    Some(parts)
}
