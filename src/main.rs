use cyanotype::*;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() {
    let f = std::fs::File::open("out.mkv").unwrap();
    let mut demuxer = Demuxer::from_seek(f).unwrap();

    let mut video_stream = demuxer.subscribe_to_video(0).unwrap();
    let mut subtitle_stream = demuxer.subscribe_to_subtitles(1).unwrap();
    let demuxer_task = tokio::task::spawn_blocking(move || {
        demuxer.run().unwrap();
    });

    loop {
        tokio::select! {
            Some(packet) = subtitle_stream.next() => {
                if let Ok(entry) = packet {
                    if let SubtitlePacket::SRTEntry { index, start, end, line } = entry {
                        println!("{}",line);
                    }
                    // if let SubtitlePacket::SSAEntry(e) = entry {
                    //      println!("{}",e.plain_text());
                    // }
                }
            },
            Some(packet) = video_stream.next() => {
                if let Ok(_p) = packet {
                    // println!("{:?}",p.time);
                }

            },
            else => break
        }
    }

    demuxer_task.await;
}
