#![allow(unused_must_use)]

use super::*;
use crate::*;
use async_trait::async_trait;

use ac_ffmpeg::codec::video::{PixelFormat, VideoDecoder, VideoFrameScaler};
use ac_ffmpeg::codec::{CodecParameters, Decoder};
use ac_ffmpeg::format::stream::Stream as FFmpegStream;
use ac_ffmpeg::packet::Packet as FFmpegPacket;
use ac_ffmpeg::time::{TimeBase as FFmpegTimeBase, Timestamp as FFmpegTimestamp};
use async_broadcast::{
    broadcast as broadcast_channel, InactiveReceiver as InactiveBroadcastReceiver,
    Receiver as BroadcastReceiver, Sender as BroadcastSender,
};
use futures::stream::Stream;
use image::RgbaImage;
use std::collections::HashMap;
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;

/// A simple video packet, containing the image data of the frame and the time where it should be presented.
#[derive(Clone)]
pub struct VideoPacket<T> {
    pub frame: T,
    pub time: Duration,
}

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
    video_transformer: VideoFrameScaler,
    height: u32,
    width: u32,
    block: bool,
    tx: BroadcastSender<VideoPacket<RgbaImage>>,
    rx: InactiveBroadcastReceiver<VideoPacket<RgbaImage>>,
}

#[async_trait]
impl PacketStream for VideoStream {
    type Packet = VideoPacket<RgbaImage>;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, rx) = broadcast_channel(64);
        let video_decoder = VideoDecoder::from_stream(stream)?.build()?;
        let video_params = parameters.as_video_codec_parameters().unwrap();
        let (width, height) = (video_params.width(), video_params.height());

        let video_transformer = VideoFrameScaler::builder()
            .source_pixel_format(video_params.pixel_format())
            .source_height(height)
            .source_width(width)
            .target_height(height)
            .target_width(width)
            .target_pixel_format(PixelFormat::from_str("rgba").unwrap())
            .build()?;

        Ok(VideoStream {
            height: height as u32,
            width: width as u32,
            metadata: stream.metadata_dict(),
            time_base: stream.time_base(),
            start_time: stream.start_time(),
            duration: stream.duration(),
            frames: stream.frames(),
            block: false,
            video_transformer,
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

    fn decode_frame(
        &mut self,
        frame: ac_ffmpeg::codec::video::frame::VideoFrame,
    ) -> Result<VideoPacket<RgbaImage>> {
        let frame = self.video_transformer.scale(&frame)?;
        Ok(VideoPacket {
            time: Duration::from_nanos(
                frame.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64
            ),
            frame: RgbaImage::from_raw(self.width, self.height, frame.planes()[0].data().to_vec())
                .ok_or(CyanotypeError::ImageDecodeError)?,
        })
    }
}

#[async_trait]
impl DecoderStream for VideoStream {
    async fn run(&mut self) -> Result<()> {
        while let Some(frame) = self.video_decoder.take()? {
            if self.block || self.tx.receiver_count() > 0 {
                let frame = self.decode_frame(frame)?;
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
