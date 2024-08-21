use std::net::SocketAddrV4;

use bytes::{Buf, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter, Result},
    net::TcpStream,
};

use std::io::Cursor;

use crate::{
    bitfield::Bitfield,
    message::{self, Message, MessageID},
};

// trait Bytable {
//     fn to_bytes(&self) -> Result<Vec<u8>, std::io::Error>;
// }

#[derive(Debug)]
pub struct PeerConnection {
    stream: BufWriter<TcpStream>,
    buffer: BytesMut,
    bitfield: Bitfield,
    choked: bool,
}

impl PeerConnection {
    pub fn new(stream: TcpStream) -> Self {
        PeerConnection {
            stream: BufWriter::new(stream),
            choked: true,
            bitfield: Bitfield::from_payload(Vec::new()),
            buffer: BytesMut::with_capacity(4 * 1024),
        }
        // !todo!();
    }

    pub async fn read_frame(&mut self) -> Result<Option<Message>> {
        loop {
            if let Some(frame) = self.parse_frame()? {
                if frame.id == MessageID::MsgBitfield {
                    self.bitfield = Bitfield::from_payload(frame.payload.clone());
                }

                // println!("{:?}", frame.id);

                if frame.id == MessageID::MsgPiece {
                    // 'inner: loop {
                    //     let n = self.stream.read_buf(&mut self.buffer).await?;

                    //     println!("{}", n);

                    //     if n != 0 {
                    //         println!("merge {} bytes", n);

                    //         let mut combined_vec = Vec::new();

                    //         combined_vec.extend(&frame.payload);
                    //         combined_vec.extend(&self.buffer[0..n]);

                    //         frame.payload = combined_vec
                    //     } else {
                    //         // empty buffer
                    //         println!("break");
                    //         break 'inner;
                    //     }
                    // }
                    // println!("piece message {}", self.buffer.len());
                }

                if frame.id == MessageID::MsgUnchoke {
                    self.choked = false;
                }
                
                println!("{:?} len: {:?}", frame.id, frame.payload.len());
                return Ok(Some(frame));
            }

            let n = self.stream.read_buf(&mut self.buffer).await?;

            println!("n size: {}", n);

            // if self.buffer.len() == self.buffer.capacity() {
            //     println!("grow {}, capacity {}", self.buffer.len(), self.buffer.capacity());
            //     self.buffer.resize(self.buffer.capacity() * 2, 0);
            // }

            // There is not enough buffered data to read a frame. Attempt to
            // read more data from the socket.
            //
            // On success, the number of bytes is returned. `0` indicates "end
            // of stream".
            if n == 0 {
                // The remote closed the connection. For this to be a clean
                // shutdown, there should be no data in the read buffer. If
                // there is, this means that the peer closed the socket while
                // sending a frame.
                if self.buffer.is_empty() {
                    return Ok(None);
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "connection reset by peer",
                    ));
                }
            }
        }
    }

    pub async fn write_frame(&mut self, message: Message) -> Result<()> {
        // implementation here
        let data = message.to_bytes()?;

        self.stream.write_all(&data).await?;
        self.stream.flush().await?;

        Ok(())
    }

    fn parse_frame(&mut self) -> Result<Option<Message>> {
        let mut buf = Cursor::new(&self.buffer[..]);

        // Parse the frame
        match Message::parse(&mut buf) {
            Ok(frame) => {
                let len = buf.position() as usize;

                // Reset the internal cursor for the
                // call to `parse`.
                buf.set_position(0);

                self.buffer.advance(len);

                // Return the frame to the caller.
                Ok(frame)
            }
            Err(error) => {
                if let message::ParsingError::Incomplete(n) = error {
                    self.buffer.reserve(n);
                }

                Ok(None)
                // match error {
                //     message::ParsingError::Incomplete(n) => {
                //         self.buffer.reserve(n);
                //         Ok(None)
                //     },
                //     message::ParsingError::Other(_) => {
                //         Ok(None)
                //     }
                // }
            },
            // Err(_) => Ok(None),
        }
    }
}
