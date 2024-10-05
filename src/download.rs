use std::path::Path;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, Result};
use serde_json;

#[derive(Serialize, Deserialize, Debug)]
pub struct DownloadState {
    file_name: String,
    // total_size: u64,
    total: usize,
    recived: Vec<usize>,
}

impl DownloadState {
    pub fn new(file_name: &str, total: usize) -> Self {
      DownloadState { file_name: file_name.to_owned(), total, recived: Vec::with_capacity(total)}
    }

    pub async fn load() -> Result<Self> {
      let mut state_file = File::open(Path::new("state.json")).await?;
      let mut state_data = String::new();
      state_file.read_to_string(&mut state_data).await?;
      let state = serde_json::from_str::<DownloadState>(&state_data)?;

      Ok(state)
    }

    pub fn get_missed_parts(&self) -> Vec<usize> {
      if self.total == 0 {
        (0..self.total).collect::<Vec<usize>>()
      } else {
        (0..self.total).filter(|n| !self.recived.contains(n)).collect::<Vec<usize>>()
      }
    }

    pub fn is_recived(&self, index: &usize) -> bool {
      self.recived.contains(index)
    }

    pub async fn update(&mut self, n: usize) -> Result<()>{
      self.recived.push(n);
      self.save().await?;

      Ok(())
    }

    async fn save(&self) -> Result<()>{
      let mut state_file = File::create(Path::new("state.json")).await?;
      let json_string = serde_json::to_string_pretty(&self)?;
      state_file.write_all(json_string.as_bytes()).await?;

      Ok(())
    }
}
