use std::{
  fmt,
  ops::Deref,
};

pub(crate) const BLOCK_LEN: u32 = 0x4000;

pub type PieceIndex = usize;

#[derive(Debug)]
pub struct Block {
  /// The index of the piece of which this is a block.
  pub piece_index: PieceIndex,
  /// The zero-based byte offset into the piece.
  pub offset: u32,
  /// The actual raw data of the block.
  pub data: BlockData,
}

impl Block {
  /// Constructs a new block based on the metadata and data.
  pub fn new(info: BlockInfo, data: impl Into<BlockData>) -> Self {
      Self {
          piece_index: info.piece_index,
          offset: info.offset,
          data: data.into(),
      }
  }

  /// Returns a [`BlockInfo`] representing the metadata of this block.
  pub fn info(&self) -> BlockInfo {
      BlockInfo {
          piece_index: self.piece_index,
          offset: self.offset,
          len: self.data.len() as u32,
      }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BlockInfo {
    /// The index of the piece of which this is a block.
    pub piece_index: PieceIndex,
    /// The zero-based byte offset into the piece.
    pub offset: u32,
    /// The block's length in bytes. Always 16 KiB (0x4000 bytes) or less, for
    /// now.
    pub len: u32,
}

impl BlockInfo {
    /// Returns the index of the block within its piece, assuming the default
    /// block length of 16 KiB.
    pub fn index_in_piece(&self) -> usize {
        // we need to use "lower than or equal" as this may be the last block in
        // which case it may be shorter than the default block length
        debug_assert!(self.len <= BLOCK_LEN);
        debug_assert!(self.len > 0);
        (self.offset / BLOCK_LEN) as usize
    }
}

impl fmt::Display for BlockInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(piece: {} offset: {} len: {})",
            self.piece_index, self.offset, self.len
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct BlockData {
  data: Vec<u8>
}

impl Deref for BlockData {
  type Target = [u8];
  fn deref(&self) -> &[u8] {
      self.data.as_ref()
  }
}

impl From<Vec<u8>> for BlockData {
  fn from(b: Vec<u8>) -> Self {
      Self { data: b }
  }
}