//! Bootstrap, GET_MESSAGE polling, and the main companion read loop.

use crate::channel::{
    author_name_from_channel_text, channel_matches_monitored, clamp_meshcore_utf8, get_channel_cmd,
    hops_label, looks_like_our_reply, message_is_exactly_test, parse_channel_info,
    parse_channel_message, parse_device_info_max_channels,     send_channel_msg_cmd, MONITORED_HASHTAGS,
};
use crate::config::{
    contact_resync_interval_secs, is_bot_enabled, poll_interval_secs, reply_location_text,
};
use crate::map_contacts;
use crate::packet_log::companion_frame_send;
use crate::protocol::{
    APP_START, DEVICE_QUERY, GET_MESSAGE, PKT_CHANNEL_INFO, PKT_CHANNEL_MSG, PKT_CHANNEL_MSG_V3,
    PKT_DEVICE_INFO, PKT_ERROR, PKT_MSG_WAITING, PKT_NO_MORE_MSGS, PKT_SELF_INFO,
};
use crate::transport::MeshTransport;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::mpsc::Sender as StdSender;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::MissedTickBehavior;

pub fn spawn_get_message_poll_std(write_tx: StdSender<Vec<u8>>) {
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

pub fn spawn_get_message_poll_unbounded(write_tx: UnboundedSender<Vec<u8>>) {
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

async fn sync_contacts(
    hub: &crate::visor::VisorHub,
    mode: map_contacts::MapLoadMode,
) -> Result<()> {
    let records = map_contacts::fetch_map_contacts(mode).await?;
    let raw = records.len();
    hub.bulk_replace_contacts(records);
    let retrieved = hub.contact_count_deduped();
    eprintln!("visor: map contact sync complete: {retrieved} contacts ({raw} rows ingelezen)");
    Ok(())
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
            if !is_bot_enabled() {
                return Ok(());
            }
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
    hub: &crate::visor::VisorHub,
) -> Result<()> {
    for _ in 0..128 {
        companion.send_payload(GET_MESSAGE).await?;
        let pkt = companion.next_frame().await?;
        hub.process_frame(&pkt);
        match pkt.first().copied() {
            Some(PKT_NO_MORE_MSGS) => return Ok(()),
            _ => handle_incoming_frame(companion, monitored, &pkt).await?,
        }
    }
    eprintln!("warning: drain stopped after 128 rounds");
    Ok(())
}

async fn wait_for_packet<T: MeshTransport + ?Sized>(
    companion: &mut T,
    want: u8,
    hub: &crate::visor::VisorHub,
) -> Result<Vec<u8>> {
    loop {
        let f = companion.next_frame().await?;
        hub.process_frame(&f);
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

async fn bootstrap<T: MeshTransport + ?Sized>(
    companion: &mut T,
    hub: &crate::visor::VisorHub,
) -> Result<HashSet<u8>> {
    companion.send_payload(APP_START).await?;
    let _self_info = wait_for_packet(companion, PKT_SELF_INFO, hub).await?;

    companion.send_payload(DEVICE_QUERY).await?;
    let dev = wait_for_packet(companion, PKT_DEVICE_INFO, hub).await?;
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
            hub.process_frame(&pkt);
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

    if let Err(e) = sync_contacts(hub, map_contacts::MapLoadMode::PreferCache).await {
        eprintln!("warning: contact sync: {e}");
    }

    Ok(monitored)
}

pub async fn run_bot_inner<T: MeshTransport>(
    transport: &mut T,
    transport_label: &str,
    hub: &crate::visor::VisorHub,
) -> Result<()> {
    eprintln!("{transport_label}");
    let monitored = bootstrap(transport, hub).await?;
    drain_pending_messages(transport, &monitored, hub).await?;

    let sync_secs = contact_resync_interval_secs();
    eprintln!("map API contact resync every {sync_secs}s");
    let mut contact_interval = tokio::time::interval(Duration::from_secs(sync_secs));
    contact_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    contact_interval.tick().await;

    loop {
        tokio::select! {
            frames = transport.read_frames() => {
                let frames = frames?;
                for frame in frames {
                    hub.process_frame(&frame);
                    handle_incoming_frame(transport, &monitored, &frame).await?;
                }
            }
            _ = contact_interval.tick() => {
                if let Err(e) = sync_contacts(hub, map_contacts::MapLoadMode::NetworkRefresh).await {
                    eprintln!("warning: periodic contact sync: {e}");
                }
            }
        }
    }
}
