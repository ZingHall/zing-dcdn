use serde::{Deserialize, Serialize};
use std::io;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const ZING_CDN_BLOB_PROTOCOL: &str = "/zing-cdn/data/1.0";
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRequest {
    pub blob_id: [u8; 32],
    pub version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobResponse {
    pub size: Option<u64>,
    #[serde(with = "serde_bytes")]
    pub data: Option<Vec<u8>>,
}

impl BlobResponse {
    pub fn have(data: Vec<u8>) -> Self {
        Self { size: Some(data.len() as u64), data: Some(data) }
    }

    pub fn not_found() -> Self {
        Self { size: None, data: None }
    }

    pub fn is_have(&self) -> bool {
        self.data.is_some()
    }
}

#[derive(Debug, Clone, Default)]
pub struct JsonProtocolCodec;

#[async_trait]
impl libp2p::request_response::Codec for JsonProtocolCodec {
    type Protocol = &'static str;
    type Request = BlobRequest;
    type Response = BlobResponse;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_buf = [0u8; 4];
        io.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("message too large: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn read_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_buf = [0u8; 4];
        io.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("message too large: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let buf = serde_json::to_vec(&req).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = (buf.len() as u32).to_be_bytes();
        io.write_all(&len).await?;
        io.write_all(&buf).await?;
        Ok(())
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let buf = serde_json::to_vec(&res).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = (buf.len() as u32).to_be_bytes();
        io.write_all(&len).await?;
        io.write_all(&buf).await?;
        Ok(())
    }
}
