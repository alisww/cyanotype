//! cyanotype is a small adapter interface to ac-ffmpeg, providing parsing, color conversion, and an async stream interface.

mod err;
pub use err::*;
pub mod streams;
pub use streams::*;

use ac_ffmpeg::codec::MediaType as FFmpegMediaType;
use ac_ffmpeg::format::demuxer::{
    Demuxer as FFmpegDemuxer, DemuxerWithStreamInfo as FFmpegDemuxerWithStreamInfo,
};
use ac_ffmpeg::format::io::IO as FFmpegIO;
use futures::stream::Stream;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::pin::Pin;

type BoxedSubtitleStream = Box<dyn PacketStream<Packet = SubtitlePacket> + Send>;
type BoxedDataStream = Box<dyn PacketStream<Packet = DataPacket> + Send>;

/// An interface over an ffmpeg demuxer.
pub struct Demuxer<T: Read + Send> {
    inner: FFmpegDemuxer<T>,
    /// A map of video streams, where the key is the stream index.
    pub video_streams: HashMap<usize, VideoStream>,
    /// A map of subtitle streams, where the key is the stream index.
    pub subtitle_streams: HashMap<usize, BoxedSubtitleStream>,
    /// A map of Raw data streams, where the key is the stream index.
    pub data_streams: HashMap<usize, BoxedDataStream>,
}

impl<T: Read + Seek + Send> Demuxer<T> {
    /// Create a Demuxer from a seekable read stream, which may allow more metadata to be read.
    pub fn from_seek(io: T) -> Result<Demuxer<T>> {
        Demuxer::from_ffmpeg(
            FFmpegDemuxer::builder()
                .build(FFmpegIO::from_seekable_read_stream(io))?
                .find_stream_info(None)
                .map_err(|v| v.1)?,
        )
    }
}

impl<T: Read + Send> Demuxer<T> {
    /// Create a Demuxer from a read stream. Streams may contain less metadata.
    pub fn from_read(io: T) -> Result<Demuxer<T>> {
        Demuxer::from_ffmpeg(
            FFmpegDemuxer::builder()
                .build(FFmpegIO::from_read_stream(io))?
                .find_stream_info(None)
                .map_err(|v| v.1)?,
        )
    }

    /// Create a Demuxer from a pre-built underlying FFmpegDemuxer
    pub fn from_ffmpeg(demuxer: FFmpegDemuxerWithStreamInfo<T>) -> Result<Demuxer<T>> {
        let mut video_streams: HashMap<usize, VideoStream> = HashMap::new();
        let mut subtitle_streams: HashMap<usize, BoxedSubtitleStream> = HashMap::new();
        let mut data_streams: HashMap<usize, BoxedDataStream> = HashMap::new();

        for (idx, stream) in demuxer.streams().iter().enumerate() {
            let params = stream.codec_parameters();
            match params.media_type() {
                FFmpegMediaType::Video => {
                    video_streams.insert(idx, VideoStream::from_ffmpeg(stream)?);
                }
                FFmpegMediaType::Subtitle => {
                    if let Some(format) = params.decoder_name() {
                        match format {
                            "ssa" | "ass" => {
                                subtitle_streams
                                    .insert(idx, Box::new(SSAStream::from_ffmpeg(stream)?));
                            }
                            "srt" => {
                                subtitle_streams
                                    .insert(idx, Box::new(SRTStream::from_ffmpeg(stream)?));
                            }
                            _ => {
                                // If codec is not known, create a passthrough decoder.
                                subtitle_streams.insert(
                                    idx,
                                    Box::new(UnknownSubtitleStream::from_ffmpeg(stream)?),
                                );
                            }
                        }
                    }
                }
                _ => {
                    data_streams.insert(idx, Box::new(DataStream::from_ffmpeg(stream)?));
                }
            }
        }

        Ok(Demuxer {
            inner: demuxer.into_demuxer(),
            video_streams,
            subtitle_streams,
            data_streams,
        })
    }

    /// Returns an async stream of [streams::VideoPacket]s from a specific stream. If the video stream specified by the index is not found, returns none.
    pub fn subscribe_to_video(
        &self,
        idx: usize,
    ) -> Option<Pin<Box<dyn Stream<Item = VideoPacket>>>> {
        self.video_streams.get(&idx).map(|v| v.stream())
    }

    /// Returns an async stream of [streams::SubtitlePacket]s from a specific stream. If the subtitle stream specified by the index is not found, returns none.
    pub fn subscribe_to_subtitles(
        &self,
        idx: usize,
    ) -> Option<Pin<Box<dyn Stream<Item = SubtitlePacket>>>> {
        self.subtitle_streams.get(&idx).map(|v| v.stream())
    }

    /// Returns an async stream of [streams::DataPacket]s from a specific stream. If the data stream specified by the index is not found, returns none.
    pub fn subscribe_to_data(&self, idx: usize) -> Option<Pin<Box<dyn Stream<Item = DataPacket>>>> {
        self.data_streams.get(&idx).map(|v| v.stream())
    }

    /// Run the decoder until it reaches an EOF.
    pub async fn run(&mut self) -> Result<()> {
        while let Some(packet) = self.inner.take()? {
            let idx = packet.stream_index();
            if let Some(stream) = self.video_streams.get_mut(&idx) {
                stream.push(packet).await?;
                stream.run().await?; // run decoder until no frames are left
            } else if let Some(stream) = self.subtitle_streams.get_mut(&idx) {
                stream.push(packet).await?;
            } else if let Some(stream) = self.data_streams.get_mut(&idx) {
                stream.push(packet).await?;
            }
        }
        // if no packets are left, run video decoders until they're done.
        for stream in self.video_streams.values_mut() {
            stream.flush()?;
            stream.run().await?;
            stream.close();
        }

        for stream in self.subtitle_streams.values() {
            stream.close();
        }

        for stream in self.data_streams.values() {
            stream.close();
        }

        Ok(())
    }
}
