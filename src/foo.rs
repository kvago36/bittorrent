use rand::prelude::*;
use sha1::{Digest, Sha1};
use url::Url;
use urlencoding::encode_binary;

use crate::{
    download::DownloadState,
    peers::Peers,
    torrent::{Info, Keys, Torrent},
    PeersInfo,
};

#[derive(Debug)]
pub struct Foo {
    // peers: Peers,
    state: DownloadState,
    torrent: Torrent,
    peer_id: [u8; 20],
    peers_info: Option<PeersInfo>,
}

impl Foo {
    fn new(torrent: Torrent, state: Option<DownloadState>) -> Self {
        let mut data = [0u8; 20];

        rand::thread_rng().fill_bytes(&mut data);

        let state = if let Some(state) = state {
          state
        } else {
          DownloadState::new(&torrent.info.name, torrent.info.pieces.0.len())
        };

        Foo {
            torrent,
            // peers,
            state,
            peers_info: None,
            peer_id: data,
        }
    }
    pub async fn get_peers(&mut self) {
        let mut url = Url::parse(&self.torrent.announce).unwrap();

        let result = Foo::info_hash(&self.torrent.info);
        let urlencoded = Foo::urlencode(&result);

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

        if let Ok(peers_info) = serde_bencode::from_bytes::<PeersInfo>(&body) {
            self.peers_info = Some(peers_info);
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
