use std::{
  fmt,
  ops::Deref,
};
#[repr(C)]
// #[repr(packed)]
// #[derive(Debug)]
pub struct Piece {
    index: usize,
    begin: usize,
    block: Vec<u8>,
}

pub(crate) const BLOCK_LEN: u32 = 0x4000;

type PieceIndex = usize;

pub struct Block {
  /// The index of the piece of which this is a block.
  pub piece_index: PieceIndex,
  /// The zero-based byte offset into the piece.
  pub offset: u32,
  /// The actual raw data of the block.
  pub data: BlockData,
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

pub struct BlockData {
  data: Vec<u8>
}

impl Deref for BlockData {
  type Target = [u8];
  fn deref(&self) -> &[u8] {
      self.data.as_ref()
      // match self {
      //     Self::Owned(b) => b.as_ref(),
      //     Self::Cached(b) => b.as_ref(),
      // }
  }
}

impl From<Vec<u8>> for BlockData {
  fn from(b: Vec<u8>) -> Self {
      Self { data: b }
  }
}

impl Piece {
  pub fn new(bytes: &[u8]) -> Self {
      let mut index_bytes = [0u8; 4];
      let mut begin_bytes = [0u8; 4];

      index_bytes.copy_from_slice(&bytes[..4]);
      begin_bytes.copy_from_slice(&bytes[..4]);

      Self {
          index: u32::from_be_bytes(index_bytes) as usize,
          begin: u32::from_be_bytes(begin_bytes) as usize,
          block: bytes[8..].to_vec(),
      }
  }

  pub fn index(&self) -> usize {
    self.index
  }

  pub fn begin(&self) -> usize {
    self.begin
  }

  pub fn to_bytes(&self) -> Vec<u8> {
      // let bytes = self.block.clone();

      let field2_copy = unsafe { std::ptr::read_unaligned(&self.block) };

      field2_copy
  }
}