use std::io;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const ZING_CDN_BLOB_PROTOCOL: &str = "/zing-cdn/data/3.0";
pub const ZING_CDN_RANGE_PROTOCOL: &str = "/zing-cdn/range/2.0";
pub const ZING_CDN_SLIVER_PROTOCOL: &str = "/zing-cdn/sliver/1.0";
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

#[derive(Debug, Clone)]
pub struct BlobRequest {
    pub blob_id: [u8; 32],
    pub version: u8,
    pub payment_tx_digest: [u8; 32],
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
        if len > MAX_MESSAGE_SIZE || len < 65 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid request length: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        let mut blob_id = [0u8; 32];
        blob_id.copy_from_slice(&buf[..32]);
        let mut payment_tx_digest = [0u8; 32];
        payment_tx_digest.copy_from_slice(&buf[33..65]);
        Ok(BlobRequest { blob_id, version: buf[32], payment_tx_digest })
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
        // 65 bytes: 32 blob_id + 1 version + 32 payment_tx_digest
        write_u32_le(io, 65).await?;
        let mut buf = [0u8; 65];
        buf[..32].copy_from_slice(&req.blob_id);
        buf[32] = req.version;
        buf[33..65].copy_from_slice(&req.payment_tx_digest);
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

#[derive(Debug, Clone)]
pub struct RangeRequest {
    pub blob_id: [u8; 32],
    pub offset: u64,
    pub length: u64,
    pub payment_tx_digest: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct RangeResponse {
    pub data: Option<Vec<u8>>,
}

impl RangeResponse {
    pub fn have(data: Vec<u8>) -> Self {
        Self { data: Some(data) }
    }

    pub fn not_found() -> Self {
        Self { data: None }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RangeProtocolCodec;

#[async_trait]
impl libp2p::request_response::Codec for RangeProtocolCodec {
    type Protocol = &'static str;
    type Request = RangeRequest;
    type Response = RangeResponse;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len != 80 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid range request length: {len} (expected 80)")));
        }
        let mut buf = [0u8; 80];
        io.read_exact(&mut buf).await?;
        let mut blob_id = [0u8; 32];
        blob_id.copy_from_slice(&buf[..32]);
        let offset = u64::from_le_bytes([buf[32], buf[33], buf[34], buf[35], buf[36], buf[37], buf[38], buf[39]]);
        let length = u64::from_le_bytes([buf[40], buf[41], buf[42], buf[43], buf[44], buf[45], buf[46], buf[47]]);
        let mut payment_tx_digest = [0u8; 32];
        payment_tx_digest.copy_from_slice(&buf[48..80]);
        Ok(RangeRequest { blob_id, offset, length, payment_tx_digest })
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
            0 => Ok(RangeResponse::not_found()),
            1 => {
                let data = buf[1..].to_vec();
                Ok(RangeResponse::have(data))
            }
            s => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown status: {s}"))),
        }
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_u32_le(io, 80).await?;
        let mut buf = [0u8; 80];
        buf[..32].copy_from_slice(&req.blob_id);
        buf[32..40].copy_from_slice(&req.offset.to_le_bytes());
        buf[40..48].copy_from_slice(&req.length.to_le_bytes());
        buf[48..80].copy_from_slice(&req.payment_tx_digest);
        io.write_all(&buf).await?;
        Ok(())
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match res.data {
            Some(data) => {
                let total_len = 1 + data.len() as u32;
                write_u32_le(io, total_len).await?;
                io.write_all(&[1]).await?;
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

#[derive(Debug, Clone)]
pub struct SliverRequest {
    pub blob_id: [u8; 32],
    pub sliver_pair_index: u16,
    pub axis: u8,
}

impl SliverRequest {
    pub const AXIS_PRIMARY: u8 = 0;
    pub const AXIS_SECONDARY: u8 = 1;
}

#[derive(Debug, Clone)]
pub struct SliverResponse {
    pub data: Option<Vec<u8>>,
}

impl SliverResponse {
    pub fn have(data: Vec<u8>) -> Self {
        Self { data: Some(data) }
    }

    pub fn not_found() -> Self {
        Self { data: None }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SliverProtocolCodec;

#[async_trait]
impl libp2p::request_response::Codec for SliverProtocolCodec {
    type Protocol = &'static str;
    type Request = SliverRequest;
    type Response = SliverResponse;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len != 35 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid sliver request length: {len} (expected 35)")));
        }
        let mut buf = [0u8; 35];
        io.read_exact(&mut buf).await?;
        let mut blob_id = [0u8; 32];
        blob_id.copy_from_slice(&buf[..32]);
        let sliver_pair_index = u16::from_le_bytes([buf[32], buf[33]]);
        let axis = buf[34];
        Ok(SliverRequest { blob_id, sliver_pair_index, axis })
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
            0 => Ok(SliverResponse::not_found()),
            1 => {
                let data = buf[1..].to_vec();
                Ok(SliverResponse::have(data))
            }
            s => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown status: {s}"))),
        }
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_u32_le(io, 35).await?;
        let mut buf = [0u8; 35];
        buf[..32].copy_from_slice(&req.blob_id);
        buf[32..34].copy_from_slice(&req.sliver_pair_index.to_le_bytes());
        buf[34] = req.axis;
        io.write_all(&buf).await?;
        Ok(())
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match res.data {
            Some(data) => {
                let total_len = 1 + data.len() as u32;
                write_u32_le(io, total_len).await?;
                io.write_all(&[1]).await?;
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

pub const ZING_CDN_ADDR_PROTOCOL: &str = "/zing-cdn/addr/1.0";

#[derive(Debug, Clone)]
pub struct AddrRequest {
    pub peer_id: libp2p::PeerId,
}

#[derive(Debug, Clone)]
pub struct AddrResponse {
    pub addresses: Vec<libp2p::Multiaddr>,
}

impl AddrResponse {
    pub fn found(addrs: Vec<libp2p::Multiaddr>) -> Self {
        Self { addresses: addrs }
    }

    pub fn not_found() -> Self {
        Self { addresses: vec![] }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AddrProtocolCodec;

#[async_trait]
impl libp2p::request_response::Codec for AddrProtocolCodec {
    type Protocol = &'static str;
    type Request = AddrRequest;
    type Response = AddrResponse;

    async fn read_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len < 1 || len > 256 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid addr request length: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        let peer_id = libp2p::PeerId::from_bytes(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid PeerId bytes: {e}")))?;
        Ok(AddrRequest { peer_id })
    }

    async fn read_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let len = read_u32_le(io).await? as usize;
        if len > MAX_MESSAGE_SIZE || len < 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid addr response length: {len}")));
        }
        let mut buf = vec![0u8; len];
        io.read_exact(&mut buf).await?;
        match buf[0] {
            0 => Ok(AddrResponse::not_found()),
            1 => {
                if len < 3 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "addr response too short"));
                }
                let count = u16::from_le_bytes([buf[1], buf[2]]) as usize;
                let mut pos = 3usize;
                let mut addresses = Vec::with_capacity(count);
                for _ in 0..count {
                    if pos + 2 > len {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "addr response truncated"));
                    }
                    let addr_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
                    pos += 2;
                    if pos + addr_len > len {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "addr response truncated"));
                    }
                    if let Ok(addr) = libp2p::Multiaddr::try_from(buf[pos..pos + addr_len].to_vec()) {
                        addresses.push(addr);
                    }
                    pos += addr_len;
                }
                Ok(AddrResponse::found(addresses))
            }
            s => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown addr status: {s}"))),
        }
    }

    async fn write_request<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, req: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let peer_bytes = req.peer_id.to_bytes();
        write_u32_le(io, peer_bytes.len() as u32).await?;
        io.write_all(&peer_bytes).await?;
        Ok(())
    }

    async fn write_response<T>(&mut self, _protocol: &Self::Protocol, io: &mut T, res: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        if res.addresses.is_empty() {
            write_u32_le(io, 1).await?;
            io.write_all(&[0]).await?;
        } else {
            let mut payload = vec![1u8];
            let count = res.addresses.len().min(u16::MAX as usize) as u16;
            payload.extend_from_slice(&count.to_le_bytes());
            for addr in &res.addresses {
                let bytes = addr.to_vec();
                let len = bytes.len().min(u16::MAX as usize) as u16;
                payload.extend_from_slice(&len.to_le_bytes());
                payload.extend_from_slice(&bytes[..len as usize]);
            }
            write_u32_le(io, payload.len() as u32).await?;
            io.write_all(&payload).await?;
        }
        Ok(())
    }
}
