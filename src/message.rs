use bytes::Buf;
use std::io::{Cursor, Read};

// use tokio::io::Result;

#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageID,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageID {
    MsgChoke = 0,
    MsgUnchoke = 1,
    MsgInterested = 2,
    MsgNotInterested = 3,
    MsgHave = 4,
    MsgBitfield = 5,
    MsgRequest = 6,
    MsgPiece = 7,
    MsgCancel = 8,
}

pub const MAX: usize = 1 << 16;
pub const BLOCK_MAX: usize = 1 << 14;

impl Message {
    pub fn new(id: MessageID, payload: Vec<u8>) -> Self {
        Message { id, payload }
    }

    pub fn to_bytes(self) -> Result<Vec<u8>, std::io::Error> {
        if self.payload.len() + 1 > MAX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large.", self.payload.len()),
            ));
        }
        let len_slice = u32::to_be_bytes(self.payload.len() as u32 + 1);
        let mut message = Vec::from(len_slice);

        // message.reserve(4 + 1 + self.payload.len());
        message.push(self.id as u8);

        message.extend_from_slice(&self.payload);

        // m: [0, 0, 0, 13]
        // m: [0, 0, 0, 13, 6]
        // m: [0, 0, 0, 13, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0]

        Ok(message)
    }

    pub fn parse(src: &mut Cursor<&[u8]>) -> Result<Option<Message>, tokio::io::Error> {        
        // Read length marker.
        if !src.has_remaining() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "empty buffer"));
        }

        let mut length_bytes = [0u8; 4];

        for i in 0..4 {
            length_bytes[i] = src.get_u8();
        }

        let length = u32::from_be_bytes(length_bytes) as usize;

        // if length == 0 {
        //     // this is a heartbeat message.
        //     // discard it.
        //     src.advance(4);
        //     // and then try again in case the buffer has more messages
        //     return self.parse(src);
        // }

        if !src.has_remaining() {
            // Not enough data to read tag marker.
            return Ok(None);
        }

        // Check that the length is not too large to avoid a denial of
        // service attack where the server runs out of memory.
        if length > MAX {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large.", length),
            ));
        }

        let tag = src.get_u8();

        let message_id = match tag {
            0 => MessageID::MsgChoke,
            1 => MessageID::MsgUnchoke,
            2 => MessageID::MsgInterested,
            3 => MessageID::MsgNotInterested,
            4 => MessageID::MsgHave,
            5 => MessageID::MsgBitfield,
            6 => MessageID::MsgRequest,
            7 => MessageID::MsgPiece,
            8 => MessageID::MsgCancel,
            message_id => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unknown message type {}.", message_id),
                ))
            }
        };

        let mut data = Vec::new();

        // read the whole file
        src.read_to_end(&mut data)?;

        Ok(Some(Message::new(message_id, data)))
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

pub struct Bitfield {
    payload: Vec<u8>
}

impl Bitfield {
    pub fn has_piece(&self, index: usize) -> bool {
        let byte_index = index / (u8::BITS as usize);
        let offset = (index % (u8::BITS as usize)) as u32;
        let Some(&byte) = self.payload.get(byte_index) else {
            return false;
        };
        byte & 1u8.rotate_right(offset + 1) != 0
    }

    pub(crate) fn pieces(&self) -> impl Iterator<Item = usize> + '_ {
        self.payload.iter().enumerate().flat_map(|(byte_i, byte)| {
            (0..u8::BITS).filter_map(move |bit_i| {
                let piece_i = byte_i * (u8::BITS as usize) + (bit_i as usize);
                let mask = 1u8.rotate_right(bit_i + 1);
                (byte & mask != 0).then_some(piece_i)
            })
        })
    }

    fn from_payload(payload: Vec<u8>) -> Bitfield {
        Self { payload }
    }
}