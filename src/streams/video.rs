#![allow(unused_must_use)]

use super::*;
use crate::*;

use ac_ffmpeg::codec::video::{PixelFormat, VideoDecoder, VideoFrameScaler};
use ac_ffmpeg::codec::{CodecParameters, Decoder};
use ac_ffmpeg::format::stream::Stream as FFmpegStream;
use ac_ffmpeg::packet::Packet as FFmpegPacket;
use ac_ffmpeg::time::{TimeBase as FFmpegTimeBase, Timestamp as FFmpegTimestamp};
use image::RgbaImage;
use std::collections::HashMap;
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::broadcast::{self, Receiver as BroadcastReceiver, Sender as BroadcastSender};
use tokio_stream::{wrappers::BroadcastStream, Stream};
/// A simple video packet, containing the image data of the frame and the time where it should be presented.
#[derive(Clone)]
pub struct VideoPacket {
    pub frame: RgbaImage,
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
    tx: BroadcastSender<VideoPacket>,
}

impl PacketStream for VideoStream {
    type Packet = VideoPacket;

    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self> {
        let parameters = stream.codec_parameters();
        let extra_data = parameters.extradata().map(|v| v.to_vec());

        let (tx, _) = broadcast::channel(64);
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
            video_transformer,
            video_decoder,
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
        self.video_decoder
            .push(packet)
            .map_err(CyanotypeError::FFmpegError)
    }
}

impl DecoderStream for VideoStream {
    fn run(&mut self) -> Result<()> {
        while let Some(frame) = self.video_decoder.take()? {
            let frame = self.video_transformer.scale(&frame)?;
            self.tx.send(VideoPacket {
                time: Duration::from_nanos(
                    frame.pts().as_nanos().ok_or(CyanotypeError::TimeMissing)? as u64,
                ),
                frame: RgbaImage::from_raw(
                    self.width,
                    self.height,
                    frame.planes()[0].data().to_vec(),
                )
                .ok_or(CyanotypeError::ImageDecodeError)?,
            });
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.video_decoder.flush()?;
        Ok(())
    }
}
