use std::collections::HashSet;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::time::Duration;

use bitfield::Bitfield;
use tokio::fs::{File, OpenOptions};
use tokio::io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

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
use foo::{Foo, Piece};
use tokio::sync::Mutex;
use torrent::Torrent;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let num = num_cpus::get();
    let mut f = File::open("debian.torrent").await?;
    let mut buffer = Vec::new();
    let state = DownloadState::load().await;

    let (blocks_tx, mut block_rx) = mpsc::unbounded_channel::<Piece>();

    f.read_to_end(&mut buffer).await?;

    let deserialized = serde_bencode::from_bytes::<Torrent>(&buffer)?;

    let plength = deserialized.info.plength;

    let mut output_f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&deserialized.info.name)
        .await?;

    let (s, r) = mpsc::unbounded_channel::<SocketAddrV4>();

    let mut state = if let Ok(state) = state {
        state
      } else {
        DownloadState::new(&deserialized.info.name, deserialized.info.pieces.0.len())
      };

    // let foo = Arc::new(Foo::new(deserialized, state.ok(), blocks_tx));
    let foo = Arc::new(Foo::new(deserialized, None, blocks_tx));

    let peers_info = foo.get_info().await.unwrap();

    // let s_clone = s.clone();
    let vec = HashSet::from_iter(state.get_missed_parts());

    tokio::spawn(async move {
        for peer in peers_info.get_peers() {
            s.send(peer).unwrap();
        }
    });

    // let d = HashSet::from_iter(state.get_missed_parts());

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
                },
                Err(_) => p.response.send(None).unwrap(),
            }
        }
    });

    foo.blabla(r, vec).await;

    // interval_handler.await.unwrap();

    // if let Ok(peers_info) = foo.get_peers().await {
    //     for peer in peers_info.get_peers() {
    //         s.send(peer).unwrap();
    //     }
    // }

    // foo.blabla().await;

    // foo.get_peers().await;
    // foo.tete().await;

    // let handler = tokio::spawn(async move {
    //     while let Some(p) = block_rx.recv().await {
    //         let offset = (p.piece_index * plength) as u32;

    //         let result: io::Result<()> = async {
    //             output_f.seek(SeekFrom::Start(offset as u64)).await?;
    //             output_f.write_all(&p.data).await?;
    //             Ok(())
    //         }
    //         .await;

    //         match result {
    //             Ok(_) => p.response.send(Some(p.piece_index)).unwrap(),
    //             Err(_) => p.response.send(None).unwrap(),
    //         }
    //     }
    // });

    // foo.download().await;

    handler.await.unwrap();

    Ok(())
}
