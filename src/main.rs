use std::collections::HashMap;
use std::io::{self};
use std::sync::Arc;

use piece::Block;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use bytes::BufMut;

use rand::prelude::*;
use sha1::{Digest, Sha1};
use url::Url;
use urlencoding::encode_binary;

use serde::Deserialize;
use serde_bencode;

mod bitfield;
mod handshake;
mod hashes;
mod message;
mod peer;
mod peers;
mod piece;
mod torrent;
// mod worker;

use handshake::Handshake;
use message::{Message, MessageId, BLOCK_MAX};
use peer::PeerConnection;
use peers::Peers;
use torrent::{Info, Keys, Torrent};

pub fn urlencode(t: &[u8; 20]) -> String {
    let mut encoded = String::with_capacity(3 * t.len());
    for &byte in t {
        encoded.push('%');
        encoded.push_str(&hex::encode(&[byte]));
    }
    encoded
}

pub fn info_hash(info: &Info) -> [u8; 20] {
    let info_encoded = bendy::serde::to_bytes(&info).unwrap();

    let mut hasher = Sha1::new();
    hasher.update(&info_encoded);
    hasher
        .finalize()
        .try_into()
        .expect("GenericArray<_, 20> == [_; 20]")
}

async fn write_piece(file: &mut File, piece: &Block, offset: u32) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset as u64)).await?;
    file.write_all(&*piece.data).await?;
    Ok(())
}

#[derive(Clone, Deserialize, Debug)]
struct PeersInfo {
    #[serde(serialize_with = "ordered_map")]
    peers: Peers,
    interval: usize,
}

#[derive(PartialEq, Debug)]
enum PieceStatus {
    Awaiting,
    Requested,
    Downloaded,
}

#[derive(Debug, PartialEq)]
struct PieceInfo {
    status: PieceStatus,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut f = File::open("sample.torrent").await?;
    let mut o = File::create("foo.txt").await?;
    let mut buffer = Vec::new();

    let (tx, mut rx) = mpsc::unbounded_channel();

    let map = Arc::new(Mutex::new(HashMap::new()));

    f.read_to_end(&mut buffer).await?;

    let deserialized = serde_bencode::from_bytes::<Torrent>(&buffer).unwrap();

    let mut url = Url::parse(&deserialized.announce).unwrap();

    map.lock()
        .await
        .extend((0..deserialized.info.pieces.0.len()).map(|n| {
            (
                n,
                PieceInfo {
                    status: PieceStatus::Awaiting,
                },
            )
        }));


    let mut data = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut data);

    let info = serde_bencode::to_bytes(&deserialized.info).unwrap();

    let mut hasher = Sha1::new();

    hasher.update(info);

    let result = info_hash(&deserialized.info);
    let urlencoded = urlencode(&result);

    // println!(
    //     "pieces_len: {:?}, psize: {:?} length: {:?}",
    //     deserialized.info.pieces.0.len(),
    //     deserialized.info.plength,
    //     deserialized.info.keys
    // );

    let coded = encode_binary(&data);

    let query_string = format!("info_hash={}&peer_id={}", urlencoded, coded);

    url.set_query(Some(&query_string));

    url.query_pairs_mut().append_pair("port", "6881");
    url.query_pairs_mut().append_pair("uploaded", "0");
    url.query_pairs_mut().append_pair("downloaded", "0");
    url.query_pairs_mut().append_pair("compact", "1");

    let length = if let Keys::SingleFile { length } = deserialized.info.keys {
        length
    } else {
        todo!()
    };

    url.query_pairs_mut()
        .append_pair("left", &length.to_string());

    let body = reqwest::get(url).await.unwrap().bytes().await.unwrap();

    let peers_info = serde_bencode::from_bytes::<PeersInfo>(&body).unwrap();

    let mut handles = Vec::new();

    for peer in peers_info.peers.0.into_iter() {
        let peer_tx = tx.clone();
        let map_clone = Arc::clone(&map);

        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(peer).await.unwrap();
            let mut handshake = Handshake::new(result, data);

            {
                let handshake_bytes =
                    &mut handshake as *mut Handshake as *mut [u8; std::mem::size_of::<Handshake>()];
                // Safety: Handshake is a POD with repr(c) and repr(packed)
                let handshake_bytes: &mut [u8; std::mem::size_of::<Handshake>()] =
                    unsafe { &mut *handshake_bytes };
                stream.write_all(handshake_bytes).await.unwrap();
                stream.read_exact(handshake_bytes).await.unwrap();
                println!("send handshake to {}", peer);
            }

            let mut pc = PeerConnection::new(stream);

            loop {
                if let Some(frame) = pc.read_frame().await.unwrap() {
                    match frame {
                        Message::Bitfield(b) => {
                            let msg_len = MessageId::Interested.header_len();
                            let mut buffer = vec![];
                            let len_slice = u32::to_be_bytes(msg_len as u32);

                            buffer.extend_from_slice(&len_slice);
                            buffer.put_u8(MessageId::Interested as u8);

                            pc.write_frame(&buffer).await.unwrap();
                        }
                        Message::Choke => todo!(),
                        Message::Unchoke => {
                            let mut count = 0;
                            let msg_len = MessageId::Request.header_len();
                            let length = u32::to_be_bytes(msg_len as u32);

                            for (id, info) in map_clone.lock().await.iter_mut() {
                                if info.status == PieceStatus::Awaiting {
                                    let index = *id as u32;
                                    let pieces_count = 3;
                                    // let pieces_count = deserialized.info.pieces.0.len() as u32;
                                    let piece_size = if index == pieces_count - 1 {
                                        // println!("p {}", deserialized.length());
                                        let md = 92063 % BLOCK_MAX;
                                        if md == 0 {
                                            BLOCK_MAX
                                        } else {
                                            md
                                        }
                                    } else {
                                        BLOCK_MAX
                                    };

                                    let blocks_count = (deserialized.info.plength + (BLOCK_MAX - 1)) / BLOCK_MAX;

                                    for i in 0..blocks_count {
                                        let block_size = if i == blocks_count - 1 {
                                            let md = piece_size % BLOCK_MAX;
                                            if md == 0 {
                                                BLOCK_MAX
                                            } else {
                                                md
                                            }
                                        } else {
                                            BLOCK_MAX
                                        };

                                        let mut buffer = vec![];
                                        let piece_index = u32::to_be_bytes(index);
                                        let offset = u32::to_be_bytes((i * BLOCK_MAX) as u32);
                                        let block_length = u32::to_be_bytes(block_size as u32);


                                        buffer.extend_from_slice(&length);
                                        buffer.put_u8(MessageId::Request as u8);

                                        buffer.extend_from_slice(&piece_index);
                                        buffer.extend_from_slice(&offset);
                                        buffer.extend_from_slice(&block_length);

                                        // println!(
                                        //     "Awaiting piece: {}, block: {}, offset: {:?}, size: {}, count: {}, peer: {}",
                                        //     index, i, i * BLOCK_MAX, block_size, count, peer
                                        // );

                                        info.status = PieceStatus::Requested;
                                        count += 1;

                                        pc.write_frame(&buffer).await.unwrap();
                                    }
                                }
                            }
                        }
                        Message::Interested => {}
                        Message::NotInterested => todo!(),
                        Message::Have { piece_index } => todo!(),
                        Message::Request(_) => todo!(),
                        Message::Piece {
                            piece_index,
                            offset,
                            data,
                        } => {
                            let block = Block {
                                piece_index,
                                offset,
                                data,
                            };

                            // println!("get piece index: {} offset: {}", piece_index, offset);
                            peer_tx.send(block).unwrap();
                        }
                        Message::Cancel(_) => todo!(),
                    }
                }
            }
        }));
    }

    while let Some(piece) = rx.recv().await {
        let offset = (piece.piece_index * deserialized.info.plength) as u32 + piece.offset;
        // println!("index: {}, offset: {}", piece.piece_index, offset);
        write_piece(&mut o, &piece, offset).await.unwrap();
    }

    for handle in handles {
        handle.await.unwrap();
    }

    Ok(())
}
