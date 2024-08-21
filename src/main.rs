use std::mem;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::{
    io::{self, Cursor, Read, Write},
};

use piece::{Block, Piece, BlockInfo};
use tokio::io::{SeekFrom, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::fs::File;
use tokio::time::Duration;

use crossbeam::channel::unbounded;

use std::sync::{Arc, Mutex};

use rand::prelude::*;
use sha1::{Digest, Sha1};
use url::Url;
use urlencoding::encode_binary;

// use bendy::decoding::{Decoder, Object};
use serde::{Deserialize, Serialize};
use serde_bencode;

mod handshake;
mod hashes;
mod message;
mod peer;
mod peers;
mod piece;
mod bitfield;

use handshake::Handshake;
use hashes::Hashes;
use message::{Message, MessageID, Request, BLOCK_MAX};
use peer::PeerConnection;
use peers::Peers;

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

const PIECE_SIZE: usize = 1 << 14; // 2^14 = 16384 bytes (16 KB)

async fn write_piece(file: &mut File, piece: &Block) -> io::Result<()> {
    // Рассчитываем смещение, куда нужно записать данную часть
    // let offset = (piece.index() * PIECE_SIZE) as u64;
    file.seek(SeekFrom::Start(0)).await?;
    file.write_all(&*piece.data).await?;
    Ok(())
}

// #[derive(Serialize, Deserialize, PartialEq, Debug)]
// struct BencodeInfo<'a> {
//     pieces: &'a [u8],
//     #[serde(rename = "piece length")]
//     piece_length: i64,
//     length: i64,
//     // files: Option<Vec<BencodeFiles>>,
//     name: String,
// }

// #[derive(Serialize, Deserialize, Debug)]
// struct BencodeFiles {
//     length: usize,
//     path: String,
// }

#[derive(Clone, Deserialize, Debug)]
struct PeersInfo {
    #[serde(serialize_with = "ordered_map")]
    peers: Peers,
    interval: usize,
}

// #[derive(Serialize, Deserialize, Debug)]
// struct Peer {
//     id: Option<String>,
//     ip: IpAddr,
//     port: i64,
// }

// #[derive(Serialize, Deserialize, Debug)]
// struct BencodeTorrent<'a> {
//     announce: String,
//     #[serde(borrow)]
//     info: BencodeInfo<'a>,
//     // comment: String,
// }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Torrent {
    /// The URL of the tracker.
    pub announce: String,

    pub info: Info,
}

#[derive(Serialize, Deserialize, Debug)]
struct SendInfo {
    info_hash: String,
    peer_id: String,
    port: SocketAddr,
    uploaded: String,
    downloaded: String,
    compact: String,
    left: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Info {
    /// The suggested name to save the file (or directory) as. It is purely advisory.
    ///
    /// In the single file case, the name key is the name of a file, in the muliple file case, it's
    /// the name of a directory.
    pub name: String,

    /// The number of bytes in each piece the file is split into.
    ///
    /// For the purposes of transfer, files are split into fixed-size pieces which are all the same
    /// length except for possibly the last one which may be truncated. piece length is almost
    /// always a power of two, most commonly 2^18 = 256K (BitTorrent prior to version 3.2 uses 2
    /// 20 = 1 M as default).
    #[serde(rename = "piece length")]
    pub plength: usize,

    /// Each entry of `pieces` is the SHA1 hash of the piece at the corresponding index.
    pub pieces: Hashes,

    #[serde(flatten)]
    pub keys: Keys,
}

/// There is a key `length` or a key `files`, but not both or neither.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Keys {
    /// If `length` is present then the download represents a single file.
    SingleFile {
        /// The length of the file in bytes.
        length: usize,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut f = File::open("sample.torrent").await?;
    let mut o = File::create("foo.txt").await?;
    let mut buffer = Vec::new();

    f.read_to_end(&mut buffer).await?;

    let deserialized = serde_bencode::from_bytes::<Torrent>(&buffer).unwrap();

    // rt.block_on(async {
    let mut url = Url::parse(&deserialized.announce).unwrap();

    // for hash in &deserialized.info.pieces.0 {
    //     println!("{}", hex::encode(&hash));
    // }

    let mut data = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut data);

    let info = serde_bencode::to_bytes(&deserialized.info).unwrap();

    let mut hasher = Sha1::new();

    hasher.update(info);

    let result = info_hash(&deserialized.info);
    let urlencoded = urlencode(&result);

    println!("pieces_len: {:?}, psize: {:?} length: {:?}", deserialized.info.pieces.0.len(), deserialized.info.plength, deserialized.info.keys);

    let coded = encode_binary(&data);

    let query_string = format!("info_hash={}&peer_id={}", urlencoded, coded);

    url.set_query(Some(&query_string));

    url.query_pairs_mut().append_pair("port", "6881");
    url.query_pairs_mut().append_pair("uploaded", "0");
    url.query_pairs_mut().append_pair("downloaded", "0");
    url.query_pairs_mut().append_pair("compact", "1");

    // println!("{}", url.to_string());

    let length = if let Keys::SingleFile { length } = deserialized.info.keys {
        length
    } else {
        todo!()
    };

    url.query_pairs_mut()
        .append_pair("left", &length.to_string());
    // deserialized.info

    let body = reqwest::get(url).await.unwrap().bytes().await.unwrap();

    // println!("body = {body:?}");

    let peers_info = serde_bencode::from_bytes::<PeersInfo>(&body).unwrap();

    // println!("{:?}", peers_info);

    let mut handles = Vec::new();

    let (tx_res, mut rx_res) = mpsc::channel(32);
    // let (tx_req, rx_req) = unbounded();

    // let mut foo = false;

    // for peer in peers_info.peers.0.into_iter() {
        // if foo == true {
        //     continue;
        // }

        // foo = true;
        // let ss = tx_res.clone();
        // let rr = rx_req.clone();

        // let pp = Peer::new(host_ip, stream);
        let test = peers_info.peers.0[0];
        // let test = SocketAddrV4::new(Ipv4Addr::new(178, 62, 85, 20), 51489);

        handles.push(tokio::spawn(async move {
            let mut stream = TcpStream::connect(test).await.unwrap();
            let mut handshake = Handshake::new(result, data);

            {
                let handshake_bytes =
                    &mut handshake as *mut Handshake as *mut [u8; std::mem::size_of::<Handshake>()];
                // Safety: Handshake is a POD with repr(c) and repr(packed)
                let handshake_bytes: &mut [u8; std::mem::size_of::<Handshake>()] =
                    unsafe { &mut *handshake_bytes };
                // println!("{:?}", handshake_bytes);
                stream.write_all(handshake_bytes).await.unwrap();
                stream.read_exact(handshake_bytes).await.unwrap();
                println!("send handshake to {}", test);
            }

            let mut pc = PeerConnection::new(stream);
            // pc.read_frame().await.unwrap();

            // let interested_message = Message::new(MessageID::MsgInterested, vec![]);
            // pc.write_frame(interested_message).await.unwrap();

            loop {
                // let message = pc.read_frame().await.unwrap();

                if let Some(frame) = pc.read_frame().await.unwrap() {
                    match frame.id {
                        MessageID::MsgBitfield => {
                            let interested_message = Message::new(MessageID::MsgInterested, vec![]);
                            pc.write_frame(interested_message).await.unwrap();
                        }
                        MessageID::MsgUnchoke => {
                            // let mut request = Request::new(0, 0, 16 * 1024);

                            // pc.get_bitfield();

                            let mut request = Request::new(
                                0 as u32,
                                (0 * BLOCK_MAX) as u32,
                                BLOCK_MAX as u32,
                            );

                            let request_bytes = Vec::from(request.as_bytes_mut());

                            let request_message = Message::new(MessageID::MsgRequest, request_bytes);

                            pc.write_frame(request_message).await.unwrap();
                            println!("send request message")
                        },
                        MessageID::MsgPiece => {
                            let block_info = BlockInfo {
                                piece_index: 0 as usize,
                                offset: 0,
                                len: frame.payload.len() as u32,
                            };

                            let block = Block::new(block_info, frame.payload);

                            // let piece = Piece::new(&frame.payload);
                            tx_res.send(block).await.unwrap();
                            // println!("{:?}", frame.payload);
                        },
                        n => !todo!()
                    }
                }
            }
        }));
    // }

    while let Some(piece) = rx_res.recv().await {
        // write to file
        println!("{}", piece.info());
        write_piece(&mut o, &piece).await.unwrap();
    }

    for handle in handles {
        handle.await.unwrap();
    }
    // });

    Ok(())
}
