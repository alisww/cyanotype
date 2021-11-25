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
use futures::stream::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

/// A packet of undecoded raw data.
#[derive(Clone)]
pub struct DataPacket {
    pub time: Option<Duration>,
    pub duration: Option<Duration>,
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
    rx: InactiveBroadcastReceiver<DataPacket>,
}

#[async_trait]
impl PacketStream for DataStream {
    type Packet = DataPacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, rx) = broadcast_channel(64);

        Ok(DataStream {
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

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet> + Send>> {
        Box::pin(self.rx.activate_cloned())
    }

    async fn push(&mut self, packet: FFmpegPacket) -> Result<()> {
        if self.tx.receiver_count() > 0 {
            let time = packet
                .pts()
                .as_nanos()
                .map(|v| Duration::from_nanos(v as u64));
            let duration = packet
                .duration()
                .as_nanos()
                .map(|v| Duration::from_nanos(v as u64));
            self.tx
                .broadcast(DataPacket {
                    data: packet.data().to_vec(),
                    time,
                    duration,
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
