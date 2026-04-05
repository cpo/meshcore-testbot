//! Companion wire constants shared across the bot and visor.

pub const FRAME_SEND_PREFIX: u8 = 0x3c; // '<'
pub const FRAME_RECV_PREFIX: u8 = 0x3e; // '>'

pub const APP_START: &[u8] = b"\x01\x03 meshcorebot";
pub const DEVICE_QUERY: &[u8] = b"\x16\x03";
pub const GET_MESSAGE: &[u8] = b"\x0a";

pub const PKT_SELF_INFO: u8 = 0x05;
/// Companion: `PACKET_CONTACT_START` / `PACKET_CONTACT` / `PACKET_CONTACT_END`.
pub const PKT_CONTACT_START: u8 = 0x02;
pub const PKT_CONTACT: u8 = 0x03;
pub const PKT_CONTACT_END: u8 = 0x04;
pub const PKT_DEVICE_INFO: u8 = 0x0d;
pub const PKT_CHANNEL_INFO: u8 = 0x12;
pub const PKT_CHANNEL_MSG: u8 = 0x08;
pub const PKT_CHANNEL_MSG_V3: u8 = 0x11;
pub const PKT_MSG_WAITING: u8 = 0x83;
/// `PACKET_LOG_DATA` — RF RX log (`MyMesh::logRxRaw`).
pub const PKT_LOG_RX_DATA: u8 = 0x88;
pub const PKT_ERROR: u8 = 0x01;
pub const PKT_NO_MORE_MSGS: u8 = 0x0a;
/// Zelfde contactpayload als `PACKET_CONTACT` (`MyMesh::writeContactRespFrame`).
pub const PUSH_NEW_ADVERT: u8 = 0x8a;
