use std::borrow::BorrowMut;
use std::collections::{HashMap, HashSet};
use std::future::IntoFuture;
use std::io::{self};
use std::net::SocketAddrV4;
use std::sync::Arc;

use bitfield::Bitfield;
use hashes::Hashes;
use piece::{Block, BlockData};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio_stream::StreamExt;

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
mod worker;
mod download;

use handshake::Handshake;
use message::{Message, MessageId, BLOCK_MAX};
use peer::PeerConnection;
use peers::Peers;
use torrent::{Info, Keys, Torrent};
use worker::ThreadPool;
use download::DownloadState;

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

pub fn piece_wrapper(b: &RequestBlock) -> Vec<u8> {
    let piece_len = MessageId::Request.header_len();
    let length = u32::to_be_bytes(piece_len as u32);
    let mut buffer = vec![];

    let piece_index = u32::to_be_bytes(b.piece_index as u32);
    let offset = u32::to_be_bytes(b.offset as u32);
    let block_length = u32::to_be_bytes(b.block_length as u32);

    buffer.extend_from_slice(&length);
    buffer.put_u8(MessageId::Request as u8);

    buffer.extend_from_slice(&piece_index);
    buffer.extend_from_slice(&offset);
    buffer.extend_from_slice(&block_length);

    buffer
    // pc.write_frame(&buffer).await.unwrap();
    // count += 1;
}

pub fn piece_parcer(index: usize, torrent: &Torrent) -> Vec<RequestBlock> {
    let pieces_count = torrent.info.pieces.0.len();
    let piece_size = if index == pieces_count - 1 {
        let md = torrent.length() % BLOCK_MAX;
        if md == 0 {
            BLOCK_MAX
        } else {
            md
        }
    } else {
        BLOCK_MAX
    };

    let blocks_count = (torrent.info.plength + (BLOCK_MAX - 1)) / BLOCK_MAX;

    (0..blocks_count)
        .map(|i| {
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
            RequestBlock {
                // peer,
                piece_index: index as u32,
                offset: i * BLOCK_MAX,
                block_length: block_size,
            }
        })
        .collect::<Vec<RequestBlock>>()
}

async fn write_piece(file: &mut File, data: &Vec<u8>, offset: u32) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset as u64)).await?;
    file.write_all(data).await?;
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
    Error,
}

#[derive(Debug)]
struct RequestBlock {
    // peer: SocketAddrV4,
    piece_index: u32,
    offset: usize,
    block_length: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut f = File::open("sample.torrent").await?;
    let mut buffer = Vec::new();
    let state = DownloadState::load().await;

    let (test_tx, mut test_rx) = mpsc::channel(100);

    // let (tx, mut rx) = mpsc::unbounded_channel();
    let (blocks_tx, mut block_rx) = mpsc::unbounded_channel::<(usize, Vec<u8>)>();

    // let map = Arc::new(Mutex::new(HashMap::new()));
    let single_map = Arc::new(Mutex::new(HashSet::new()));

    let mut kkk = HashSet::new();

    f.read_to_end(&mut buffer).await?;

    let deserialized = Arc::new(serde_bencode::from_bytes::<Torrent>(&buffer)?);

    let mut output_f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&deserialized.info.name)
        .await?;

    let mut url = Url::parse(&deserialized.announce).unwrap();

    let state = if let Ok(state) = state {
        state
    } else {
        DownloadState::new(&deserialized.info.name, deserialized.info.pieces.0.len())
    };

    kkk.extend(state.get_missed_parts());

    // kkk.extend((0..deserialized.info.pieces.0.len()).map(|n| n));

    let state = RwLock::new(state);

    single_map
        .lock()
        .await
        .extend((0..deserialized.info.pieces.0.len()).map(|n| n));

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

    // let mut handles = Vec::new();

    let peers = peers_info.peers.0.clone();
    let mut stream = tokio_stream::iter(peers);

    let fff = Arc::new(Mutex::new(HashMap::new()));

    while let Some(peer) = stream.next().await {
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

        while let Some(frame) = pc.read_frame().await.unwrap() {
            if let Message::Bitfield(b) = frame {
                // println!("{:?}", b);

                if b.pieces().any(|p| kkk.contains(&p)) {
                    fff.lock().await.insert(peer, (b, pc));
                }

                // maybe close connection or sth because peer doest have pieces to download

                break;
            }
        }
    }

    let test_tx = Arc::new(test_tx);

    for i in kkk.into_iter() {
        // let res = some_computation(i).await;
        test_tx.send(i).await.unwrap();
    }

    let pool = ThreadPool::new(2);

    let torrent = Arc::clone(&deserialized);

    tokio::spawn(async move {
        while let Some((piece_index, data)) = block_rx.recv().await {
            // println!("piece {:?}", piece);
            // println!("arc count {}", Arc::strong_count(&test_tx));
            let offset = (piece_index * torrent.info.plength) as u32;

            match write_piece(&mut output_f, &data, offset).await {
                Ok(_) => {
                    let mut n = state.write().await;

                    n.update(piece_index).await.unwrap();
                },
                Err(_) => todo!(),
            };
        }
    });

    while let Some(k) = test_rx.recv().await {
        // for k in kkk.iter() {
        // let k = k.clone();
        let blocks_tx_copy = blocks_tx.clone();
        let torrent = Arc::clone(&deserialized);
        let ff = Arc::clone(&fff);
        let piece_queue = Arc::clone(&test_tx.clone());
        pool.execute(move || {
            let rt = Runtime::new().unwrap();

            rt.block_on(async {
                let mut peers = ff.lock().await;
                let mut peer_addr = None;

                for (peer, value) in peers.iter() {
                    if value.0.has_piece(k) {
                        peer_addr = Some(peer.clone());
                        break;
                    }
                }

                let peer = if let Some(peer_addr) = peer_addr {
                    peers.remove(&peer_addr)
                } else {
                    None
                };

                drop(peers);

                if let Some((_, mut pc)) = peer {
                    // let (b, mut pc) = n.remove(&p).unwrap();

                    let msg_len = MessageId::Interested.header_len();
                    let len_slice = u32::to_be_bytes(msg_len as u32);
                    let mut buffer = vec![];
                    let mut count = 5;
                    let mut recived = 0;
                    let pieces_count = torrent.info.pieces.0.len();
                    let blocks = piece_parcer(k, &torrent);
                    let piece_size: usize = if k + 1 == pieces_count {
                        torrent.info.plength - (torrent.info.plength * pieces_count - torrent.length())
                    } else {
                        torrent.info.plength
                    };

                    let mut all_blocks = vec![0u8; piece_size];

                    buffer.extend_from_slice(&len_slice);
                    buffer.put_u8(MessageId::Interested as u8);

                    pc.write_frame(&buffer).await.unwrap();

                    loop {
                        if let Some(frame) = pc.read_frame().await.unwrap() {
                            match frame {
                                Message::Unchoke => {
                                    // println!("Unchoked {}", p);

                                    for b in blocks.iter().take(5) {
                                        let buffer = piece_wrapper(b);

                                        pc.write_frame(&buffer).await.unwrap();
                                    }
                                }
                                Message::Choke => {
                                    piece_queue.send(k).await.unwrap();
                                }
                                Message::Piece {
                                    piece_index,
                                    offset,
                                    data,
                                } => {
                                    // let (resp_tx, resp_rx) = oneshot::channel();

                                    let block = Block {
                                        piece_index,
                                        offset,
                                        data,
                                    };

                                    recived += 1;

                                    println!("get piece index: {} offset: {}", piece_index, offset);

                                    all_blocks[block.offset as usize..][..block.data.len()].copy_from_slice(&block.data);

                                    if count < blocks.len() {
                                        count += 1;
                                        let buffer = piece_wrapper(&blocks[count]);

                                        pc.write_frame(&buffer).await.unwrap();
                                    }

                                    // blocks_tx_copy.send((block, resp_tx)).unwrap();

                                    if recived == blocks.len() {
                                        // println!("length: {}", all_blocks.len());
                                        let mut hasher = Sha1::new();
                                        hasher.update(&all_blocks);
                                        let hash: [u8; 20] = hasher
                                            .finalize()
                                            .try_into()
                                            .expect("GenericArray<_, 20> == [_; 20]");

                                        let is_equal = hash.iter()
                                        .zip(torrent.info.pieces.0[k].iter())
                                        .all(|(&a, &b)| a == b );

                                        if is_equal {
                                            blocks_tx_copy.send((k, all_blocks)).unwrap();
                                            println!("great success!");
                                        } else {
                                            piece_queue.send(k).await.unwrap();
                                        }

                                        break;
                                    }
                                }
                                _ => todo!(),
                            }
                        }
                    }
                }
            })
        });
    }

    Ok(())
}
