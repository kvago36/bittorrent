use std::fmt;

use serde::{Deserialize, Serialize};

use serde_bencode::error::Error;

use crate::hashes::Hashes;

#[derive(Debug)]
// #[non_exhaustive]
pub enum TorrentError {
    ParseError,
}

impl From<Error> for TorrentError {
    fn from(_: Error) -> Self {
        // the pieces field is a concatenation of 20 byte SHA-1 hashes, so it
        // must be a multiple of 20
        Self::ParseError
    }
}

impl fmt::Display for TorrentError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use TorrentError::*;
        match self {
            ParseError => {
                write!(fmt, "cant parse torrent file")
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Torrent {
    /// The URL of the tracker.
    pub announce: String,

    pub info: Info,
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

impl Torrent {
    pub fn length(&self) -> usize {
        match &self.info.keys {
            Keys::SingleFile { length } => *length,
            // Keys::MultiFile { files } => files.iter().map(|file| file.length).sum(),
        }
    }
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
