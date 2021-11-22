#![allow(unused_must_use)]

use super::*;
use crate::*;

use ac_ffmpeg::codec::CodecParameters;
use ac_ffmpeg::format::stream::Stream as FFmpegStream;
use ac_ffmpeg::packet::Packet as FFmpegPacket;
use ac_ffmpeg::time::{TimeBase as FFmpegTimeBase, Timestamp as FFmpegTimestamp};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::watch::{self, Receiver as WatchReceiver, Sender as WatchSender};
use tokio_stream::{wrappers::WatchStream, Stream, StreamExt};

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
    tx: WatchSender<Option<SubtitlePacket>>,
}

impl PacketStream for SSAStream {
    type Packet = Option<SubtitlePacket>;

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

        let (tx, _) = watch::channel(None);

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

    fn subscribe(&self) -> WatchReceiver<Self::Packet> {
        self.tx.subscribe()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(
            tokio_stream::iter(
                self.header
                    .clone()
                    .into_iter()
                    .map(|v| Some(SubtitlePacket::SSASection(v))),
            )
            .chain(WatchStream::new(self.tx.subscribe())),
        )
    }

    fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
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
        self.tx.send(Some(SubtitlePacket::SSAEntry(entry)));
        Ok(())
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
    tx: WatchSender<Option<SubtitlePacket>>,
}

impl PacketStream for SRTStream {
    type Packet = Option<SubtitlePacket>;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, _) = watch::channel(None);

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

    fn subscribe(&self) -> WatchReceiver<Self::Packet> {
        self.tx.subscribe()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(WatchStream::new(self.tx.subscribe()))
    }

    fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
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

        self.tx.send(Some(SubtitlePacket::SRTEntry {
            index: self.index,
            start: time,
            end: time + duration,
            line: String::from_utf8(packet.data().to_vec())?,
        }));
        Ok(())
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
    tx: WatchSender<Option<SubtitlePacket>>,
}

impl PacketStream for UnknownSubtitleStream {
    type Packet = Option<SubtitlePacket>;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, _) = watch::channel(None);

        Ok(UnknownSubtitleStream {
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            extra_data,
            parameters,
            tx,
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

    fn subscribe(&self) -> WatchReceiver<Self::Packet> {
        self.tx.subscribe()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>> {
        Box::pin(WatchStream::new(self.tx.subscribe()))
    }

    fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        let time = Duration::from_nanos(
            packet.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
        );
        let duration = Duration::from_nanos(
            packet
                .duration()
                .as_nanos()
                .ok_or(CyanotypeError::TimeMissing)? as u64,
        );

        self.tx.send(Some(SubtitlePacket::Raw {
            start: time,
            end: time + duration,
            data: packet.data().to_vec(),
        }));
        Ok(())
    }
}
