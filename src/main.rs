use std::{fs::{self, File}, io::{self, Read}};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::runtime::Runtime;
use url::Url;
use rand::prelude::*;
use urlencoding::encode_binary;
use sha1::{Sha1, Digest};

use bendy::decoding::{Decoder, Object};
use serde::{Deserialize, Serialize};

pub use hashes::Hashes;

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

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct BencodeInfo<'a> {
    pieces: &'a [u8], 
    #[serde(rename = "piece length")]
    piece_length: i64,  
    length: i64,
    // files: Option<Vec<BencodeFiles>>,    
    name: String, 
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct BencodeFiles {
    length: usize,
    path: String
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct BencodeTorrent<'a> {
    announce: String,
    #[serde(borrow)]
    info: BencodeInfo<'a>,
    // comment: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Torrent {
    /// The URL of the tracker.
    pub announce: String,

    pub info: Info,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct SendInfo {
    info_hash:  String,
    peer_id: String,
    port: SocketAddr,
    uploaded:  String,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut f = File::open("sample.torrent")?;
    let mut buffer = Vec::new();

    f.read_to_end(&mut buffer)?;

    let deserialized = bendy::serde::from_bytes::<Torrent>(&buffer).unwrap();

    let rt  = Runtime::new()?;

    rt.block_on(async {
        let mut url = Url::parse(&deserialized.announce).unwrap();

        for hash in &deserialized.info.pieces.0 {
            println!("{}", hex::encode(&hash));
        }

        let mut data = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut data);

        let info = bendy::serde::to_bytes(&deserialized.info).unwrap();

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

        // println!("{}", url.to_string());

        let length = if let Keys::SingleFile { length } = deserialized.info.keys {
            length
        } else {
            todo!()
        };

        url.query_pairs_mut().append_pair("left", &length.to_string());
        // deserialized.info

        let body = reqwest::get(url)
            .await.unwrap()
            .text()
            .await.unwrap();

        println!("body = {body:?}");
    });

    Ok(())
}


mod hashes {
    use serde::de::{self, Deserialize, Deserializer, Visitor};
    use serde::ser::{Serialize, Serializer};
    use std::fmt;

    #[derive(Debug, Clone)]
    pub struct Hashes(pub Vec<[u8; 20]>);
    struct HashesVisitor;

    impl<'de> Visitor<'de> for HashesVisitor {
        type Value = Hashes;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a byte string whose length is a multiple of 20")
        }

        fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v.len() % 20 != 0 {
                return Err(E::custom(format!("length is {}", v.len())));
            }
            // TODO: use array_chunks when stable
            Ok(Hashes(
                v.chunks_exact(20)
                    .map(|slice_20| slice_20.try_into().expect("guaranteed to be length 20"))
                    .collect(),
            ))
        }
    }

    impl<'de> Deserialize<'de> for Hashes {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_bytes(HashesVisitor)
        }
    }

    impl Serialize for Hashes {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let single_slice = self.0.concat();
            serializer.serialize_bytes(&single_slice)
        }
    }
}