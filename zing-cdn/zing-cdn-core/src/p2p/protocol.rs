use std::io;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const ZING_CDN_BLOB_PROTOCOL: &str = "/zing-cdn/data/2.0";
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone)]
pub struct BlobRequest {
    pub blob_id: [u8; 32],
    pub version: u8,
}

#[derive(Debug, Clone)]
pub struct BlobResponse {
    pub size: Option<u64>,
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
pub struct BinaryProtocolCodec;

#[async_trait]
impl libp2p::request_response::Codec for BinaryProtocolCodec {
    type Protocol = &'static str;
    type Request = BlobRequest;
    type Response = BlobResponse;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len > MAX_MESSAGE_SIZE || len < 33 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid request length: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        let mut blob_id = [0u8; 32];
        blob_id.copy_from_slice(&buf[..32]);
        Ok(BlobRequest { blob_id, version: buf[32] })
    }

    async fn read_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len > MAX_MESSAGE_SIZE || len < 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid response length: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        match buf[0] {
            0 => Ok(BlobResponse::not_found()),
            1 => {
                if len < 5 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "have response too short"));
                }
                let data_len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
                if len != 5 + data_len {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "have response length mismatch"));
                }
                let data = buf[5..].to_vec();
                Ok(BlobResponse::have(data))
            }
            s => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown status: {s}"))),
        }
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        // 33 bytes: 32 blob_id + 1 version
        write_u32_le(io, 33).await?;
        let mut buf = [0u8; 33];
        buf[..32].copy_from_slice(&req.blob_id);
        buf[32] = req.version;
        io.write_all(&buf).await?;
        Ok(())
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match res.data {
            Some(data) => {
                let data_len = data.len() as u32;
                write_u32_le(io, 5 + data_len).await?;
                io.write_all(&[1]).await?;
                io.write_all(&data_len.to_le_bytes()).await?;
                io.write_all(&data).await?;
            }
            None => {
                write_u32_le(io, 1).await?;
                io.write_all(&[0]).await?;
            }
        }
        Ok(())
    }
}

async fn read_u32_le<T: AsyncRead + Unpin + Send>(io: &mut T) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    io.read_exact(&mut buf).await?;
    Ok(u32::from_le_bytes(buf))
}

async fn write_u32_le<T: AsyncWrite + Unpin + Send>(io: &mut T, val: u32) -> io::Result<()> {
    io.write_all(&val.to_le_bytes()).await
}
