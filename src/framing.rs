//! `<` / `>` length-prefixed companion framing.

use crate::protocol::{FRAME_RECV_PREFIX, FRAME_SEND_PREFIX};

/// Build the wire frame for an outbound companion payload (no logging).
pub fn encode_companion_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut v = Vec::with_capacity(3 + payload.len());
    v.push(FRAME_SEND_PREFIX);
    v.extend_from_slice(&len.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub struct RxFramer {
    header: Vec<u8>,
    expected: usize,
    inframe: Vec<u8>,
}

impl RxFramer {
    pub fn new() -> Self {
        Self {
            header: Vec::new(),
            expected: 0,
            inframe: Vec::new(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
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
            if self.expected > 2048 {
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
