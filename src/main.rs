use std::collections::HashSet;
use std::sync::Arc;

use tokio::fs::{File, OpenOptions};
use tokio::io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::mpsc::{self};

use serde_bencode;

mod bitfield;
mod download;
mod foo;
mod handshake;
mod hashes;
mod message;
mod peer;
mod peers;
mod piece;
mod torrent;
mod worker;

use download::DownloadState;
use foo::{Foo, Piece, RequestPiece};
use tokio::sync::{oneshot, Semaphore};
use torrent::Torrent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut f = File::open("debian.torrent").await?;
    let mut buffer = Vec::new();
    let state = DownloadState::load().await;

    let (blocks_tx, mut block_rx) = mpsc::unbounded_channel::<Piece>();
    let (queue_tx, queue_rx) = mpsc::channel(100);

    f.read_to_end(&mut buffer).await?;

    let deserialized = serde_bencode::from_bytes::<Torrent>(&buffer)?;

    let plength = deserialized.info.plength;

    let mut output_f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&deserialized.info.name)
        .await?;

    let mut state = if let Ok(state) = state {
        state
    } else {
        DownloadState::new(&deserialized.info.name, deserialized.info.pieces.0.len())
    };

    let vec: HashSet<usize> = HashSet::from_iter(state.get_missed_parts());

    let mut foo = Foo::new(deserialized, queue_rx, blocks_tx);

    let handler = tokio::spawn(async move {
        while let Some(p) = block_rx.recv().await {
            let offset = (p.piece_index * plength) as u32;

            let result: io::Result<()> = async {
                output_f.seek(SeekFrom::Start(offset as u64)).await?;
                output_f.write_all(&p.data).await?;
                Ok(())
            }
            .await;

            match result {
                Ok(_) => {
                    state.update(p.piece_index).await.unwrap();
                    p.response.send(Some(p.piece_index)).unwrap()
                }
                Err(_) => p.response.send(None).unwrap(),
            }
        }
    });

    let semaphore = Arc::new(Semaphore::new(10));

    for k in vec.clone() {
        let semaphore = semaphore.clone();
        let cmd_tx = queue_tx.clone();

        tokio::spawn(async move {
            let (resp_tx, resp_rx) = oneshot::channel();

            let _permit = semaphore.acquire().await.unwrap();

            cmd_tx
                .send(RequestPiece::new(k, resp_tx))
                .await
                .ok()
                .unwrap();
            
            // TODO: called `Result::unwrap()` on an `Err` value: RecvError(())
            let res = resp_rx.await.unwrap();

            drop(_permit);
        });
    }

    foo.download(vec).await.unwrap();

    handler.await.unwrap();

    Ok(())
}
