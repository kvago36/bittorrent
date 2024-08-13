#[derive(Debug)]
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

  pub fn from_payload(payload: Vec<u8>) -> Bitfield {
      Self { payload }
  }
}