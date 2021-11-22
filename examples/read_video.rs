use cyanotype::*;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    let f = std::fs::File::open("out.mkv").unwrap();
    let mut demuxer = Demuxer::from_seek(f).unwrap();

    let mut video_stream = demuxer.subscribe_to_video(0).unwrap();
    let mut subtitle_stream = demuxer.subscribe_to_subtitles(2).unwrap();
    let demuxer_task = tokio::task::spawn(async move {
        demuxer.run().await.unwrap();
    });

    let mut idx = 0;
    let time = tokio::time::Instant::now();

    loop {
        tokio::select! {
            Some(packet) = video_stream.next() => {
                // if let Ok(_p) = packet {
                idx += 1;
                // }
                println!("{:?}",packet.time);

            },
            else => break
        }
    }

    let took = time.elapsed();
    demuxer_task.await;
}
