use thiserror::Error;

#[derive(Error, Debug)]
pub enum CyanotypeError {
    #[error("couldn't find a videostream")]
    NoVideoStream,
    #[error("couldn't parse subtitles")]
    SubtitleError,
    #[error("couldn't decode image")]
    ImageDecodeError,
    #[error("couldn't get packet time from ffmpeg")]
    TimeMissing,
    #[error(transparent)]
    UTF8Error(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    FFmpegError(#[from] ac_ffmpeg::Error),
}

pub type Result<T> = std::result::Result<T, CyanotypeError>;
