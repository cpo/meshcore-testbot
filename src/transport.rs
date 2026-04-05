//! Serial and TCP companion transports (shared `<`/`>` framing).

use crate::framing::RxFramer;
use crate::packet_log::{companion_frame_send, log_received_packets};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::sync::mpsc::Sender as StdSender;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

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
    use std::io::{Read, Write};

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
