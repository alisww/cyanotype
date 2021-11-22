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
use tokio::sync::broadcast::{self, Receiver as BroadcastReceiver, Sender as BroadcastSender};
use tokio_stream::{wrappers::BroadcastStream, Stream};

/// A packet of undecoded raw data.
#[derive(Clone)]
pub struct DataPacket {
    pub time: Duration,
    pub data: Vec<u8>,
}

/// A raw data stream.
pub struct DataStream {
    metadata: HashMap<&'static str, &'static str>,
    time_base: FFmpegTimeBase,
    start_time: FFmpegTimestamp,
    duration: FFmpegTimestamp,
    frames: Option<u64>,
    extra_data: Option<Vec<u8>>,
    parameters: CodecParameters,
    tx: BroadcastSender<DataPacket>,
}

impl PacketStream for DataStream {
    type Packet = DataPacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, _) = broadcast::channel(64);

        Ok(DataStream {
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

    fn subscribe(&self) -> BroadcastReceiver<Self::Packet> {
        self.tx.subscribe()
    }

    fn stream(&self) -> Pin<Box<dyn Stream<Item = RecvResult<Self::Packet>>>> {
        Box::pin(BroadcastStream::new(self.tx.subscribe()))
    }

    fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        let time = Duration::from_nanos(
            packet.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
        );
        self.tx.send(DataPacket {
            data: packet.data().to_vec(),
            time,
        });
        Ok(())
    }
}
