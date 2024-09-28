use bytes::Buf;
use std::io::{Cursor, Read};

use crate::{
    bitfield::Bitfield,
    piece::{BlockData, BlockInfo},
};

#[derive(Debug)]
pub enum ParsingError {
    /// Not enough data is available to parse a message
    Incomplete(usize),

    /// Invalid message encoding
    Other(std::io::Error),
}

/// The actual messages exchanged by peers.
#[derive(Debug, PartialEq)]
pub(crate) enum Message {
    Bitfield(Bitfield),
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have {
        piece_index: usize,
    },
    Request(BlockInfo),
    Piece {
        piece_index: usize,
        offset: u32,
        data: BlockData,
    },
    Cancel(BlockInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageId {
    Choke = 0,
    Unchoke = 1,
    Interested = 2,
    NotInterested = 3,
    Have = 4,
    Bitfield = 5,
    Request = 6,
    Piece = 7,
    Cancel = 8,
}

impl MessageId {
    /// Returns the header length of the specific message type.
    ///
    /// Since this is fix size for all messages, it can be determined simply
    /// from the message id.
    pub fn header_len(&self) -> u64 {
        match self {
            Self::Choke => 1,
            Self::Unchoke => 1,
            Self::Interested => 1,
            Self::NotInterested => 1,
            Self::Have => 4 + 1 + 4,
            Self::Bitfield => 4 + 1,
            Self::Request => 1 + 3 * 4,
            Self::Piece => 1 + 2 * 4,
            Self::Cancel => 1 + 3 * 4,
        }
    }
}

pub const MAX: usize = 1 << 16;
pub const BLOCK_MAX: usize = 1 << 14;

impl Message {
    pub fn parse(src: &mut Cursor<&[u8]>) -> Result<Option<Message>, ParsingError> {
        // Read length marker.
        if src.get_ref().len() < 4 {
            return Ok(None);
        }

        let length = src.get_u32() as usize;
        
        if length == 0 {
            // This is a heartbeat message.
            return Ok(None);
        }

        if src.get_ref().len() < 4 + length {
            // We inform the Framed that we need more bytes to form the next
            // frame.
            return Err(ParsingError::Incomplete(src.get_ref().len()));
        }

        if !src.has_remaining() {
            // Not enough data to read tag marker.
            return Ok(None);
        }

        // Check that the length is not too large
        if length > MAX {
            return Err(ParsingError::Other(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large.", length),
            )));
        }

        let tag = src.get_u8();

        let message_id = match tag {
            0 => MessageId::Choke,
            1 => MessageId::Unchoke,
            2 => MessageId::Interested,
            3 => MessageId::NotInterested,
            4 => MessageId::Have,
            5 => MessageId::Bitfield,
            6 => MessageId::Request,
            7 => MessageId::Piece,
            8 => MessageId::Cancel,
            message_id => {
                return Err(ParsingError::Other(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unknown message type {}.", message_id),
                )));
            }
        };

        let message = match message_id {
            MessageId::Choke => Message::Choke,
            MessageId::Unchoke => Message::Unchoke,
            MessageId::Interested => Message::Interested,
            MessageId::NotInterested => Message::NotInterested,
            MessageId::Bitfield => {
                let mut data = Vec::new();

                // read the whole file
                src.read_to_end(&mut data).unwrap();

                let bitfield = Bitfield::from_payload(data);
                Message::Bitfield(bitfield)
            },
            MessageId::Piece => {
                let piece_index = src.get_u32() as usize;
                let offset = src.get_u32();

                let mut foo = vec![0; length - 4 - 4 - 1];
                src.read_exact(&mut foo).unwrap();

                // println!("length: {} data: {}, len: {}", length, foo.len(), src.get_ref().len());

                Message::Piece { piece_index, offset, data: foo.into() }
            },
            MessageId::Have => todo!(),
            MessageId::Request => todo!(),
            MessageId::Cancel => todo!(),
        };

        Ok(Some(message))
    }
}

#[repr(C)]
#[repr(packed)]
pub struct Request {
    index: [u8; 4],
    begin: [u8; 4],
    length: [u8; 4],
}

impl Request {
    pub fn new(index: u32, begin: u32, length: u32) -> Self {
        Self {
            index: index.to_be_bytes(),
            begin: begin.to_be_bytes(),
            length: length.to_be_bytes(),
        }
    }

    pub fn index(&self) -> u32 {
        u32::from_be_bytes(self.index)
    }

    pub fn begin(&self) -> u32 {
        u32::from_be_bytes(self.begin)
    }

    pub fn length(&self) -> u32 {
        u32::from_be_bytes(self.length)
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        let bytes = self as *mut Self as *mut [u8; std::mem::size_of::<Self>()];
        // Safety: Self is a POD with repr(c) and repr(packed)
        let bytes: &mut [u8; std::mem::size_of::<Self>()] = unsafe { &mut *bytes };
        bytes
    }
}