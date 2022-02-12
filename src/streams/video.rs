#![allow(unused_must_use)]

use super::*;
use crate::*;
use async_trait::async_trait;

use ac_ffmpeg::codec::{CodecParameters, Decoder};
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

pub use ac_ffmpeg::codec::video::{PixelFormat, VideoDecoder, VideoFrame, VideoFrameScaler};

/// A stream adapter over an underlying ffmpeg video decoder, which converts raw frames into [image::RgbaImage].
pub struct VideoStream {
    metadata: HashMap<&'static str, &'static str>,
    time_base: FFmpegTimeBase,
    start_time: FFmpegTimestamp,
    duration: FFmpegTimestamp,
    frames: Option<u64>,
    extra_data: Option<Vec<u8>>,
    parameters: CodecParameters,
    video_decoder: VideoDecoder,
    block: bool,
    tx: BroadcastSender<VideoFrame>,
    rx: InactiveBroadcastReceiver<VideoFrame>,
}

#[async_trait]
impl PacketStream for VideoStream {
    type Packet = VideoFrame;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, rx) = broadcast_channel(64);
        let video_decoder = VideoDecoder::from_stream(stream)?.build()?;
        let _video_params = parameters.as_video_codec_parameters().unwrap();

        Ok(VideoStream {
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            block: false,
            video_decoder,
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
        // if self.tx.receiver_count() > 0 {
        self.video_decoder
            .push(packet)
            .map_err(CyanotypeError::FFmpegError);
        // }
        Ok(())
    }

    fn close(&self) {
        self.tx.close();
    }
}

impl VideoStream {
    pub fn blocking(&mut self, block: bool) {
        self.block = block;
    }

    pub fn pixel_format(&self) -> PixelFormat {
        self.parameters
            .as_video_codec_parameters()
            .unwrap()
            .pixel_format()
    }
}

#[async_trait]
impl DecoderStream for VideoStream {
    async fn run(&mut self) -> Result<()> {
        while let Some(frame) = self.video_decoder.take()? {
            if self.block || self.tx.receiver_count() > 0 {
                self.tx
                    .broadcast(frame)
                    .await
                    .map_err(|_| CyanotypeError::ChannelSendError)?;
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.video_decoder.flush()?;
        Ok(())
    }
}
