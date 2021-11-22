use crate::Result;
use ac_ffmpeg::codec::CodecParameters;
use ac_ffmpeg::format::stream::Stream as FFmpegStream;
use ac_ffmpeg::packet::Packet as FFmpegPacket;
use ac_ffmpeg::time::{TimeBase as FFmpegTimeBase, Timestamp as FFmpegTimestamp};
use async_broadcast::Receiver as BroadcastReceiver;
use async_trait::async_trait;
use futures::stream::Stream;
use std::collections::HashMap;
use std::pin::Pin;

/// An adapter that converts raw ffmpeg packets and outputs them to a tokio channel or async stream.
#[async_trait]
pub trait PacketStream {
    type Packet;

    /// Create a packet stream from an underlying FFmpeg stream.
    fn from_ffmpeg(stream: &FFmpegStream) -> Result<Self>
    where
        Self: Sized;

    // ffmpeg metadata
    /// Extra codec-dependent data.
    fn extra_data(&self) -> Option<&[u8]>;
    /// Stream metadata, as a dictionary. May not be complete if the Demuxer IO is not seekable or the format doesn't have metadata.
    fn metadata(&self) -> HashMap<&'static str, &'static str>;
    /// Stream timebase, as in [ac_ffmpeg::time::TimeBase].
    fn time_base(&self) -> FFmpegTimeBase;
    /// Stream start time, with a timestamp as defined in [ac_ffmpeg::time::Timestamp].
    fn start_time(&self) -> FFmpegTimestamp;
    /// Stream duration time, with a timestamp as defined in [ac_ffmpeg::time::Timestamp].
    fn duration(&self) -> FFmpegTimestamp;
    /// Quantity of frames in this stream, if defined.
    fn frames(&self) -> Option<u64>;
    /// FFmpeg codec parameters.
    fn parameters(&self) -> CodecParameters;

    // processing
    /// Push an FFmpeg packet into this stream for decoding.
    async fn push(&mut self, packet: FFmpegPacket) -> Result<()>;
    /// Get a new receiver, which receives a message every time a new packet is decoded.
    fn subscribe(&self) -> BroadcastReceiver<Self::Packet>;
    /// Get a stream of decoded packets from this stream.
    fn stream(&self) -> Pin<Box<dyn Stream<Item = Self::Packet>>>;

    fn close(&self);
}

/// A stream that needs additional underlying encoding.
#[async_trait]
pub trait DecoderStream: PacketStream {
    /// Flushes the underlying ffmpeg decoder.
    fn flush(&mut self) -> Result<()>;
    /// Runs underlying decoder while it has packets queued.
    async fn run(&mut self) -> Result<()>;
}

mod data;
mod subtitles;
mod video;
pub use data::*;
pub use subtitles::*;
pub use video::*;
