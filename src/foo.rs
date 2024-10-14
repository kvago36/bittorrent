use std::collections::{HashMap, HashSet};
use std::net::SocketAddrV4;
use std::sync::Arc;

use rand::prelude::*;
use sha1::{Digest, Sha1};
use url::Url;
use urlencoding::encode_binary;

use bytes::BufMut;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{RwLock, Mutex};
use tokio::sync::oneshot::{self, Sender};
use tokio::time::{self, Duration};

use log::{error, info, warn};

use serde::Deserialize;

use thiserror::Error;

use crate::bitfield::Bitfield;
use crate::message::{Message, MessageId, BLOCK_MAX};
use crate::peer::PeerConnection;
use crate::peers::Peers;
use crate::piece::Block;
use crate::{
    download::DownloadState,
    handshake::Handshake,
    torrent::{Info, Keys, Torrent},
};

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
}

pub fn piece_parcer(index: usize, torrent_length: usize, pieces_count: usize, plength: usize) -> Vec<RequestBlock> {
    let piece_size = if index == pieces_count - 1 {
        let md = torrent_length % BLOCK_MAX;
        if md == 0 {
            BLOCK_MAX
        } else {
            md
        }
    } else {
        BLOCK_MAX
    };

    let blocks_count = (plength + (BLOCK_MAX - 1)) / BLOCK_MAX;

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
                piece_index: index as u32,
                offset: i * BLOCK_MAX,
                block_length: block_size,
            }
        })
        .collect::<Vec<RequestBlock>>()
}

#[derive(Debug, Error)]
pub enum FooError {
    #[error("Error! Invalid hash!")]
    InvalidHash,

    #[error("Error! No have pieces for download!")]
    Empty,

    #[error("Error! Cant writting to the file")]
    Unknown,

    #[error("Error! Peer silenced the connection")]
    Choke,

    #[error("data store disconnected")]
    Disconnect(#[from] io::Error),
}

#[derive(Debug)]
enum PeerState {
    Connected,
    Bitfield(Bitfield),
    /// count, received, requested
    Download(usize, usize, usize),
    Interested,
    Ready,
    Finished(usize),
    /// received, requested
    Awaiting(usize, usize),
}

#[derive(Clone, Deserialize, Debug)]
pub struct PeersInfo {
    #[serde(serialize_with = "ordered_map")]
    peers: Peers,
    interval: usize,
}

impl PeersInfo {
    pub fn get_peers(&self) -> Vec<SocketAddrV4> {
        self.peers.0.clone()
    }
    pub fn get_interval(&self) -> usize {
        self.interval
    }
}

#[derive(Debug)]
struct RequestBlock {
    piece_index: u32,
    offset: usize,
    block_length: usize,
}

pub struct Piece {
    pub piece_index: usize,
    pub response: Sender<Option<usize>>,
    pub data: Vec<u8>
}

#[derive(Debug)]
pub struct Foo {
    state: Arc<Mutex<DownloadState>>,
    torrent: Torrent,
    peer_id: [u8; 20],
    info_hash: [u8;20],
    pieces: Arc<HashSet<usize>>,
    connections: Arc<RwLock<HashMap<SocketAddrV4, (Bitfield, PeerConnection)>>>,
    peers_info: Option<PeersInfo>,
    sender: UnboundedSender<Piece>,
}

impl Foo {
    pub fn new(torrent: Torrent, state: Option<DownloadState>, sender: UnboundedSender<Piece>) -> Self {
        let mut data = [0u8; 20];

        rand::thread_rng().fill_bytes(&mut data);

        let state = if let Some(state) = state {
          state
        } else {
          DownloadState::new(&torrent.info.name, torrent.info.pieces.0.len())
        };

        let info_hash = Foo::info_hash(&torrent.info);

        let pieces: Arc<HashSet<usize>> = Arc::new(HashSet::from_iter(state.get_missed_parts().iter().cloned().collect::<Vec<_>>()));
    
        Foo {
            torrent,
            state: Arc::new(Mutex::new(state)),
            pieces,
            connections: Arc::new(RwLock::new(HashMap::new())),
            peers_info: None,
            peer_id: data,
            info_hash,
            sender,
        }
    }
    pub async fn tete(&self) {
        let mut join_handles = vec![];
        let peers_info = self.peers_info.clone();

        for peer in peers_info.unwrap().peers.0 {
            let ba = Arc::clone(&self.pieces);
            let connections = Arc::clone(&self.connections);
            let peer_id = self.peer_id.clone();
            let info_hash = self.info_hash.clone();

            join_handles.push(tokio::spawn(async move {
                let timeout_duration = Duration::from_secs(2);
    
                match time::timeout(timeout_duration, TcpStream::connect(peer)).await {
                    Ok(Ok(mut stream)) => {
                        let mut handshake = Handshake::new(info_hash, peer_id);

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
                                                connections.write().await.insert(peer.clone(), (bitfield, pc));
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

        info!("waiting for tasks finish");

        for join_handle in join_handles.drain(..) {
            join_handle.await.unwrap();
        };
    }
    pub async fn get_info(&self) -> Result<PeersInfo, serde_bencode::Error> {
        let mut url = Url::parse(&self.torrent.announce).unwrap();

        let urlencoded = Foo::urlencode(&self.info_hash);

        let coded = encode_binary(&self.peer_id);
        let query_string = format!("info_hash={}&peer_id={}", urlencoded, coded);

        url.set_query(Some(&query_string));

        url.query_pairs_mut().append_pair("port", "6881");
        url.query_pairs_mut().append_pair("uploaded", "0");
        url.query_pairs_mut().append_pair("downloaded", "0");
        url.query_pairs_mut().append_pair("compact", "1");

        let length = if let Keys::SingleFile { length } = self.torrent.info.keys {
            length
        } else {
            todo!()
        };

        url.query_pairs_mut()
            .append_pair("left", &length.to_string());

        let body = reqwest::get(url).await.unwrap().bytes().await.unwrap();

        serde_bencode::from_bytes::<PeersInfo>(&body)
    }
    pub async fn blabla(&self, mut r: UnboundedReceiver<SocketAddrV4>, state: HashSet<usize>) {
        let mut join_handles = vec![];

        let b_to_d = Arc::new(RwLock::new(state));

        while let Some(peer) = r.recv().await {
            let peer_id = self.peer_id.clone();
            let info_hash = self.info_hash.clone();
            let bbb = Arc::clone(&b_to_d);

            let plength = self.torrent.info.plength;
            let pieces_count = self.torrent.info.pieces.0.len();
            let torrent_length = self.torrent.length();
            let pieces_queue = self.sender.clone();
            let piecies = self.torrent.info.pieces.0.clone();

            join_handles.push(tokio::spawn(async move {
                let mut count = 0;

                'outer: loop {
                    count += 1;

                    let mut stream = TcpStream::connect(peer).await.unwrap();
                    let mut handshake = Handshake::new(info_hash, peer_id);

                    {
                        let handshake_bytes = &mut handshake as *mut Handshake as *mut [u8; std::mem::size_of::<Handshake>()];
                        // Safety: Handshake is a POD with repr(c) and repr(packed)
                        let handshake_bytes: &mut [u8; std::mem::size_of::<Handshake>()] =
                            unsafe { &mut *handshake_bytes };
                        stream.write_all(handshake_bytes).await.unwrap();
                        stream.read_exact(handshake_bytes).await.unwrap();
                        // Ok(())
                    }

                    let mut pc = PeerConnection::new(stream);
                    let mut peer_state = PeerState::Connected;

                    let mut blocks = Vec::with_capacity(pieces_count);
                    let mut piece_size = 0;
                    let mut all_blocks = Vec::new();
                    let mut pindex = None;

                    let mut available_pieces = vec![];

                    let result: Result<(), FooError> = async { loop {
                        match peer_state {
                            PeerState::Connected => (),
                            PeerState::Bitfield(bitfield) => {
                                let n = bbb.read().await;

                                let intersection: Vec<_> = bitfield.pieces()
                                    // .iter()
                                    .filter(|x| n.contains(x))
                                    .collect();

                                if intersection.len() > 0 {
                                    available_pieces = intersection;
                                    peer_state = PeerState::Interested;
                                    continue;
                                } else {
                                    break Err(FooError::Empty);
                                }
                            },
                            PeerState::Interested => {
                                let msg_len = MessageId::Interested.header_len();
                                let len_slice = u32::to_be_bytes(msg_len as u32);
                                let mut buffer = vec![];

                                buffer.extend_from_slice(&len_slice);
                                buffer.put_u8(MessageId::Interested as u8);
            
                                pc.write_frame(&buffer).await.unwrap();
                            },
                            PeerState::Ready => {
                                let random_index = {
                                    let mut rng = rand::thread_rng();
                                    rng.gen_range(0..available_pieces.len())
                                };
                                let random_value = available_pieces.get(random_index).unwrap();

                                let mut n = bbb.write().await;

                                if n.remove(random_value) {
                                    pindex = Some(random_value.to_owned());
                                } else {
                                    for (i, piece) in available_pieces.iter().enumerate() {
                                        if n.remove(piece) {
                                            pindex = Some(*piece);
                                            available_pieces.remove(i);
                                            break;
                                        }
                                    };
                                };

                                if let Some(index) = pindex {
                                    blocks = piece_parcer(index, torrent_length, pieces_count, plength);
                                    
                                    piece_size = if index + 1 == pieces_count {
                                        plength
                                            - (plength * pieces_count - torrent_length)
                                    } else {
                                        plength
                                    };
    
                                    all_blocks = vec![0u8; piece_size];
    
                                    // info!("all_blocks: {:?}, piece_size: {:?}", all_blocks.len(), piece_size);
    
                                    peer_state = PeerState::Download(5, 0, 0);
                                    continue;
                                } else {
                                    break Err(FooError::Empty);
                                }
                            },
                            PeerState::Download(count, rec, req) => {
                                let mut i = 0;

                                for b in blocks.iter().skip(req).take(count) {
                                    let buffer = piece_wrapper(b);

                                    i += 1;

                                    pc.write_frame(&buffer).await?;
                                };

                                peer_state = PeerState::Awaiting(rec, req + i);
                            },
                            PeerState::Awaiting(_, _) => (),
                            PeerState::Finished(k) => {
                                let mut hasher = Sha1::new();
                                hasher.update(&all_blocks);
                                let hash: [u8; 20] = hasher
                                    .finalize()
                                    .try_into()
                                    .expect("GenericArray<_, 20> == [_; 20]");

                                // info!("all_blocks: {:?}", all_blocks.len());

                                let is_equal = hash
                                    .iter()
                                    .zip(piecies[k].iter())
                                    .all(|(&a, &b)| a == b);

                                if is_equal {
                                    let (tx, rx) = oneshot::channel::<Option<usize>>();

                                    pieces_queue.send(
                                        Piece {
                                            piece_index: k, 
                                            response: tx,
                                            data: all_blocks
                                        }
                                    ).unwrap();

                                    if let Ok(response) = rx.await {
                                        if let Some(_) = response {
                                            info!("great success!");
                                            break Ok(());
                                        }
                                    }

                                    break Err(FooError::Unknown);
                                }

                                break Err(FooError::InvalidHash);
                            },
                        }


                        if let Some(frame) = pc.read_frame().await? {
                            match frame {
                                Message::Bitfield(bitfield) => {
                                    info!("Get bitfield from {}", peer);
                                    peer_state = PeerState::Bitfield(bitfield);
                                },
                                Message::Choke => break Err(FooError::Choke),
                                Message::Unchoke => {
                                    peer_state = PeerState::Ready;
                                },
                                Message::Interested => todo!(),
                                Message::NotInterested => todo!(),
                                Message::Have { piece_index } => todo!(),
                                Message::Request(block_info) => todo!(),
                                Message::Piece { piece_index, offset, data } => {
                                    let block = Block {
                                        piece_index,
                                        offset,
                                        data,
                                    };


                                    if let PeerState::Awaiting(received, requested) = peer_state {
                                        if requested < blocks.len() {
                                            peer_state = PeerState::Download(1, received + 1, requested);
                                        } else if received + 1 == blocks.len() {
                                            peer_state = PeerState::Finished(piece_index);
                                        } else {
                                            peer_state = PeerState::Awaiting(received + 1, requested);
                                        }
                                    }
    
                                    all_blocks[block.offset as usize..][..block.data.len()]
                                        .copy_from_slice(&block.data);

                                    // info!("piece: {}, offset: {}", piece_index, offset)
                                },
                                Message::Cancel(block_info) => todo!(),
                            }
                        }
                    }}.await;

                    match result {
                        Ok(_) => continue 'outer,
                        Err(FooError::Empty) => break 'outer,
                        Err(FooError::Choke) => {

                        },
                        Err(FooError::InvalidHash) | Err(FooError::Unknown) |
                        Err(FooError::Disconnect(_)) => {
                            if let Some(index) = pindex {
                                bbb.write().await.insert(index);
                            }

                            if count > 5 {
                                break 'outer;
                            }

                            count += 1;
                            continue 'outer;
                        },
                    }
                }
            }));
        }

        for join_handle in join_handles.drain(..) {
            join_handle.await.unwrap();
        }
    }
    fn urlencode(t: &[u8; 20]) -> String {
        let mut encoded = String::with_capacity(3 * t.len());
        for &byte in t {
            encoded.push('%');
            encoded.push_str(&hex::encode(&[byte]));
        }
        encoded
    }
    fn info_hash(info: &Info) -> [u8; 20] {
        let info_encoded = bendy::serde::to_bytes(&info).unwrap();

        let mut hasher = Sha1::new();
        hasher.update(&info_encoded);
        hasher
            .finalize()
            .try_into()
            .expect("GenericArray<_, 20> == [_; 20]")
    }
}
