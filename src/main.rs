use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use piece::{Block, BlockData};
use tokio::fs::{File, OpenOptions};
use tokio::io::{self, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{self};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::time::{self, Duration};

use bytes::BufMut;

use log::{error, info, warn};
use rand::prelude::*;
use sha1::{Digest, Sha1};
use url::Url;
use urlencoding::encode_binary;

use serde::Deserialize;
use serde_bencode;

mod bitfield;
mod download;
mod handshake;
mod hashes;
mod message;
mod peer;
mod peers;
mod piece;
mod torrent;
mod worker;
mod foo;

use foo::Foo;
use download::DownloadState;
use handshake::Handshake;
use message::{Message, MessageId, BLOCK_MAX};
use peer::PeerConnection;
use peers::Peers;
use torrent::{Info, Keys, Torrent};
use worker::ThreadPool;

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

#[derive(Debug)]
struct RequestBlock {
    // peer: SocketAddrV4,
    piece_index: u32,
    offset: usize,
    block_length: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let num = num_cpus::get();
    let mut f = File::open("debian.torrent").await?;
    let mut buffer = Vec::new();
    let state = DownloadState::load().await;


    let (test_tx, mut test_rx) = mpsc::channel(100);

    let (blocks_tx, mut block_rx) = mpsc::unbounded_channel::<(usize, Vec<u8>)>();

    let single_map = Arc::new(Mutex::new(HashSet::new()));

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

    let kkk: Arc<HashSet<usize>> = Arc::new(HashSet::from_iter(state.get_missed_parts().iter().cloned().collect::<Vec<_>>()));

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

    let fff = Arc::new(RwLock::new(HashMap::new()));

    let mut join_handles = vec![];

    for peer in peers_info.peers.0 {
        let ba = Arc::clone(&kkk);
        let bitfield_map = Arc::clone(&fff);
        join_handles.push(tokio::spawn(async move {
            let timeout_duration = Duration::from_secs(2);

            match time::timeout(timeout_duration, TcpStream::connect(peer)).await {
                Ok(Ok(mut stream)) => {
                    let mut handshake = Handshake::new(result, data);

                    let result: io::Result<()> = async {
                        let handshake_bytes = &mut handshake as *mut Handshake as *mut [u8; std::mem::size_of::<Handshake>()];
                        // Safety: Handshake is a POD with repr(c) and repr(packed)
                        let handshake_bytes: &mut [u8; std::mem::size_of::<Handshake>()] =
                            unsafe { &mut *handshake_bytes };
                        stream.write_all(handshake_bytes).await?;
                        stream.read_exact(handshake_bytes).await?;
                        Ok(())
                    }.await;
        
                    if let Ok(_) = result {
                        let mut pc = PeerConnection::new(stream);
        
                        tokio::select! {
                            Ok(Some(frame)) = pc.read_frame() => {
                                match frame {
                                    Message::Bitfield(bitfield) => {
                                        info!("{} send bitfield", peer);
                                        if bitfield.pieces().any(|p| ba.contains(&p)) {
                                            bitfield_map.write().await.insert(peer, (bitfield, pc));
                                        }
                                    },
                                    _ => {
                                        warn!("Peer didnt sent bitfield msg")
                                    }
                                }
                            }
                            _ = time::sleep(Duration::from_millis(600)) => {
                                warn!("timeout for bitfield msg")
                            }
                        }
                    } else {
                        error!("Error during handshake check")

                    }
                }
                Ok(Err(e)) => {
                    error!("Failed to connect: {:?}", e);
                }
                Err(_) => {
                    info!("Connection attempt timed out after {:?} seconds", timeout_duration);
                }
            }
        }));
    }

    let test_tx = Arc::new(test_tx);
    let pool = ThreadPool::new(num);
    let torrent = Arc::clone(&deserialized);

    tokio::spawn(async move {
        while let Some((piece_index, data)) = block_rx.recv().await {
            let offset = (piece_index * torrent.info.plength) as u32;

            match write_piece(&mut output_f, &data, offset).await {
                Ok(_) => {
                    let mut n = state.write().await;

                    n.update(piece_index).await.unwrap();
                }
                Err(_) => todo!(),
            };
        }
    });

    info!("waiting for tasks finish");

    for join_handle in join_handles.drain(..) {
        join_handle.await.unwrap();
    }

    info!("starting queue");
    let test_t = Arc::clone(&test_tx);

    tokio::spawn(async move {
        if let Ok(kkk) = Arc::try_unwrap(kkk) {
            info!("tasks to download: {:?}", kkk.len());
            for i in kkk.into_iter() {
                test_t.send(i).await.unwrap();
            }
        } else {
            error!("cant unwrap tasks");
        }
    });

    while let Some(k) = test_rx.recv().await {
        let blocks_tx_copy = blocks_tx.clone();
        let torrent = Arc::clone(&deserialized);
        let ff = Arc::clone(&fff);
        let piece_queue = Arc::clone(&test_tx);

        if let None = fff.read().await.values().find(|(b, _)| b.has_piece(k)) {
            info!("no have peers for piece {}", k);
            continue;
        }

        pool.execute(move || {
            info!("recived {} task", k);

            let rt = Runtime::new().unwrap();

            rt.block_on(async {
                let mut peers = ff.write().await;
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

                if let Some((bitfield, mut pc)) = peer {
                    let msg_len = MessageId::Interested.header_len();
                    let len_slice = u32::to_be_bytes(msg_len as u32);
                    let mut buffer = vec![];
                    let mut count = 5;
                    let mut recived = 0;
                    let mut is_finished = false;
                    let pieces_count = torrent.info.pieces.0.len();
                    let blocks = piece_parcer(k, &torrent);
                    let piece_size: usize = if k + 1 == pieces_count {
                        torrent.info.plength
                            - (torrent.info.plength * pieces_count - torrent.length())
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
                                    for b in blocks.iter().take(5) {
                                        let buffer = piece_wrapper(b);

                                        pc.write_frame(&buffer).await.unwrap();
                                    }
                                }
                                Message::Choke => {
                                    piece_queue.send(k).await.unwrap();
                                    break;
                                }
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

                                    recived += 1;

                                    // info!("get piece index: {} offset: {}", piece_index, offset);

                                    all_blocks[block.offset as usize..][..block.data.len()]
                                        .copy_from_slice(&block.data);

                                    if count < blocks.len() {
                                        count += 1;
                                        let buffer = piece_wrapper(&blocks[count - 1]);

                                        pc.write_frame(&buffer).await.unwrap();
                                    }

                                    if recived == blocks.len() {
                                        let mut hasher = Sha1::new();
                                        hasher.update(&all_blocks);
                                        let hash: [u8; 20] = hasher
                                            .finalize()
                                            .try_into()
                                            .expect("GenericArray<_, 20> == [_; 20]");

                                        let is_equal = hash
                                            .iter()
                                            .zip(torrent.info.pieces.0[k].iter())
                                            .all(|(&a, &b)| a == b);

                                        if is_equal {
                                            blocks_tx_copy.send((k, all_blocks)).unwrap();
                                            info!("great success!");
                                        } else {
                                            error!("piece hash isnt equal!");
                                            piece_queue.send(k).await.unwrap();
                                            is_finished = true;
                                        }

                                        break;
                                    }
                                }
                                _ => todo!(),
                            }
                        }
                    }

                    if is_finished {
                        ff.write().await.insert(peer_addr.unwrap(), (bitfield, pc));
                    }
                } else {
                    info!("all peers are buzy now sending piece: {} back to queue", k);
                    piece_queue.send(k).await.unwrap();
                }
            })
        });
    }

    Ok(())
}
