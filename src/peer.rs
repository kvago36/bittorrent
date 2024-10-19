use bytes::{Buf, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter, Result},
    net::TcpStream,
};

use std::io::Cursor;

use crate::{bitfield::{self, Bitfield}, message::{Message, ParsingError}};

#[derive(Debug)]
pub struct PeerConnection {
    stream: BufWriter<TcpStream>,
    buffer: BytesMut,
    choked: bool,
    bitfield: Option<Bitfield>,
}

impl PeerConnection {
    pub fn new(stream: TcpStream) -> Self {
        PeerConnection {
            stream: BufWriter::new(stream),
            buffer: BytesMut::with_capacity(4 * 1024),
            bitfield: None,
            choked: false
        }
    }

    pub async fn read_frame(&mut self) -> Result<Option<Message>> {
        loop {
            if let Some(frame) = self.parse_frame()? {
                return Ok(Some(frame));
            }

            let n = self.stream.read_buf(&mut self.buffer).await?;

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

    pub async fn write_frame(&mut self, bytes: &[u8]) -> Result<()> {
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;

        Ok(())
    }

    pub fn set_bitfield(&mut self, bitfield: Bitfield) {
        self.bitfield = Some(bitfield)
    }

    pub fn get_bitfield(&self) -> Option<&Bitfield> {
        self.bitfield.as_ref()
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
                if let ParsingError::Incomplete(n) = error {
                    self.buffer.reserve(n);
                }

                Ok(None)
            },
        }
    }
}
