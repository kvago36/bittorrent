// use crate::{hashes, peers};

// use serde::{Deserialize, Serialize};

// use hashes::Hashes;

#[repr(C)]
#[derive(Clone)]
pub struct Handshake {
  length: u8,
  pstr: [u8; 19],
  reserved: [u8; 8],
  info_hash: [u8; 20],
  peer_id: [u8; 20]
}

impl Handshake {
  pub fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
    Handshake {
      length: 19,
      pstr: *b"BitTorrent protocol",
      info_hash,
      peer_id,
      reserved: [0; 8],
    }
  }
}