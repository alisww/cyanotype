#![allow(unused_must_use)]

use super::*;
use crate::*;

use ac_ffmpeg::codec::CodecParameters;
use ac_ffmpeg::format::stream::Stream as FFmpegStream;
use ac_ffmpeg::packet::Packet as FFmpegPacket;
use ac_ffmpeg::time::{TimeBase as FFmpegTimeBase, Timestamp as FFmpegTimestamp};
use async_broadcast::{
    broadcast as broadcast_channel, InactiveReceiver as InactiveBroadcastReceiver,
    Receiver as BroadcastReceiver, Sender as BroadcastSender,
};
use futures::stream::{self, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

/// A subtitle packet.
#[derive(Debug, Clone)]
pub enum SubtitlePacket {
    /// A Substation Alpha header packet, sent at the start of a subtitle stream
    SSASection(substation::Section),
    /// A Substation Alpha subtitle entry.
    SSAEntry(substation::Entry),
    /// A SubRip subtitle entry.
    SRTEntry {
        index: usize,
        start: Duration,
        end: Duration,
        line: String,
    },
    /// A raw subtitle packet, left undecoded.
    Raw {
        start: Duration,
        end: Duration,
        data: Vec<u8>,
    },
}

/// A substation alpha decoder stream.
pub struct SSAStream {
    metadata: HashMap<&'static str, &'static str>,
    time_base: FFmpegTimeBase,
    start_time: FFmpegTimestamp,
    duration: FFmpegTimestamp,
    frames: Option<u64>,
    extra_data: Option<Vec<u8>>,
    header: Vec<substation::Section>,
    parameters: CodecParameters,
    definition_header: Vec<String>,
    tx: BroadcastSender<SubtitlePacket>,
    rx: InactiveBroadcastReceiver<SubtitlePacket>,
}

#[async_trait]
impl PacketStream for SSAStream {
    type Packet = SubtitlePacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());
        let header = if let Some(ref data) = extra_data {
            let mut input = String::from_utf8(data.clone())?;
            let mut results: Vec<substation::Section> = Vec::new();
            while let Ok((new_input, sect)) = substation::parser::section(&input) {
                results.push(sect);
                input = new_input.trim_start().to_owned();
            }

            results
        } else {
            Vec::new()
        };

        let (tx, rx) = broadcast_channel(64);

        Ok(SSAStream {
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            definition_header: vec![
                "ReadOrder",
                "Layer",
                "Style",
                "Name",
                "MarginL",
                "MarginR",
                "MarginV",
                "Effect",
                "Text",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            extra_data,
            parameters,
            tx,
            rx: rx.deactivate(),
            header,
        })
    }

    // ffmpeg metadata
    fn extra_data(&self) -> Option<&[u8]> {
        self.extra_data.as_deref()
    }

    fn metadata(&self) -> HashMap<&'static str, &'static str> {
        self.metadata.clone()
    }

    fn time_base(&self) -> FFmpegTimeBase {
        self.time_base
    }

    fn start_time(&self) -> FFmpegTimestamp {
        self.start_time
    }

    fn duration(&self) -> FFmpegTimestamp {
        self.duration
    }

    fn frames(&self) -> Option<u64> {
        self.frames
    }

    fn parameters(&self) -> CodecParameters {
        self.parameters.clone()
    }

    fn subscribe(&self) -> BroadcastReceiver<Self::Packet> {
        self.rx.activate_cloned()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(
            stream::iter(
                self.header
                    .clone()
                    .into_iter()
                    .map(SubtitlePacket::SSASection),
            )
            .chain(self.rx.activate_cloned()),
        )
    }

    async fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        if self.tx.receiver_count() > 0 {
            self.tx.set_overflow(false);

            let time = Duration::from_nanos(
                packet.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
            );
            let duration = Duration::from_nanos(
                packet
                    .duration()
                    .as_nanos()
                    .ok_or(CyanotypeError::TimeMissing)? as u64,
            );
            let (_, mut entry) = substation::parser::subtitle(
                &String::from_utf8(packet.data().to_vec())?,
                &self.definition_header,
            )
            .map_err(|_| CyanotypeError::SubtitleError)?;
            entry.start = Some(time);
            entry.end = Some(time + duration);
            self.tx
                .broadcast(SubtitlePacket::SSAEntry(entry))
                .await
                .map_err(|_| CyanotypeError::ChannelSendError)?;
        }
        Ok(())
    }

    fn close(&self) {
        self.tx.close();
    }
}

/// A SubRip decoder stream.
pub struct SRTStream {
    metadata: HashMap<&'static str, &'static str>,
    time_base: FFmpegTimeBase,
    start_time: FFmpegTimestamp,
    duration: FFmpegTimestamp,
    frames: Option<u64>,
    extra_data: Option<Vec<u8>>,
    parameters: CodecParameters,
    index: usize,
    tx: BroadcastSender<SubtitlePacket>,
    rx: InactiveBroadcastReceiver<SubtitlePacket>,
}

#[async_trait]
impl PacketStream for SRTStream {
    type Packet = SubtitlePacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, rx) = broadcast_channel(64);

        Ok(SRTStream {
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            index: 0,
            extra_data,
            parameters,
            tx,
            rx: rx.deactivate(),
        })
    }

    // ffmpeg metadata
    fn extra_data(&self) -> Option<&[u8]> {
        self.extra_data.as_deref()
    }

    fn metadata(&self) -> HashMap<&'static str, &'static str> {
        self.metadata.clone()
    }

    fn time_base(&self) -> FFmpegTimeBase {
        self.time_base
    }

    fn start_time(&self) -> FFmpegTimestamp {
        self.start_time
    }

    fn duration(&self) -> FFmpegTimestamp {
        self.duration
    }

    fn frames(&self) -> Option<u64> {
        self.frames
    }

    fn parameters(&self) -> CodecParameters {
        self.parameters.clone()
    }

    fn subscribe(&self) -> BroadcastReceiver<Self::Packet> {
        self.rx.activate_cloned()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(self.rx.activate_cloned())
    }

    async fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        if self.tx.receiver_count() > 0 {
            let time = Duration::from_nanos(
                packet.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
            );
            let duration = Duration::from_nanos(
                packet
                    .duration()
                    .as_nanos()
                    .ok_or(CyanotypeError::TimeMissing)? as u64,
            );

            self.index += 1;

            self.tx
                .broadcast(SubtitlePacket::SRTEntry {
                    index: self.index,
                    start: time,
                    end: time + duration,
                    line: String::from_utf8(packet.data().to_vec())?,
                })
                .await
                .map_err(|_| CyanotypeError::ChannelSendError)?;
        }
        Ok(())
    }

    fn close(&self) {
        self.tx.close();
    }
}

/// A pass-through subtitle stream that returns raw packets.
pub struct UnknownSubtitleStream {
    metadata: HashMap<&'static str, &'static str>,
    time_base: FFmpegTimeBase,
    start_time: FFmpegTimestamp,
    duration: FFmpegTimestamp,
    frames: Option<u64>,
    extra_data: Option<Vec<u8>>,
    parameters: CodecParameters,
    tx: BroadcastSender<SubtitlePacket>,
    rx: InactiveBroadcastReceiver<SubtitlePacket>,
}

#[async_trait]
impl PacketStream for UnknownSubtitleStream {
    type Packet = SubtitlePacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, rx) = broadcast_channel(64);

        Ok(UnknownSubtitleStream {
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            extra_data,
            parameters,
            tx,
            rx: rx.deactivate(),
        })
    }

    // ffmpeg metadata
    fn extra_data(&self) -> Option<&[u8]> {
        self.extra_data.as_deref()
    }

    fn metadata(&self) -> HashMap<&'static str, &'static str> {
        self.metadata.clone()
    }

    fn time_base(&self) -> FFmpegTimeBase {
        self.time_base
    }

    fn start_time(&self) -> FFmpegTimestamp {
        self.start_time
    }

    fn duration(&self) -> FFmpegTimestamp {
        self.duration
    }

    fn frames(&self) -> Option<u64> {
        self.frames
    }

    fn parameters(&self) -> CodecParameters {
        self.parameters.clone()
    }

    fn subscribe(&self) -> BroadcastReceiver<Self::Packet> {
        self.rx.activate_cloned()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(self.rx.activate_cloned())
    }

    async fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        if self.tx.receiver_count() > 0 {
            let time = Duration::from_nanos(
                packet.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
            );
            let duration = Duration::from_nanos(
                packet
                    .duration()
                    .as_nanos()
                    .ok_or(CyanotypeError::TimeMissing)? as u64,
            );

            self.tx
                .broadcast(SubtitlePacket::Raw {
                    start: time,
                    end: time + duration,
                    data: packet.data().to_vec(),
                })
                .await
                .map_err(|_| CyanotypeError::ChannelSendError)?;
        }
        Ok(())
    }

    fn close(&self) {
        self.tx.close();
    }
}
