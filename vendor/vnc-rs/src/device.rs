//! TrollVNC device control v1. Independent of Tight security and file transfer.
//! SetEncodings: 0xC0A1A990. Frames: type/version/op/status/id:u32/len:u32/UTF-8 JSON.
use crate::VncError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_DEVICE_PAYLOAD: usize = 1024 * 1024;
pub const MAX_DEVICE_REQUEST: usize = 4096;

#[derive(Debug, Clone)]
pub struct DeviceRequest {
    pub op: u8,
    pub id: u32,
    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct DeviceReply {
    pub op: u8,
    pub id: u32,
    pub status: u8,
    pub payload: String,
}

impl DeviceRequest {
    pub(crate) async fn write<S: AsyncWrite + Unpin>(self, writer: &mut S) -> Result<(), VncError> {
        if !(1..=10).contains(&self.op) || self.id == 0 || self.payload.len() > MAX_DEVICE_REQUEST {
            return Err(VncError::General("Invalid device request".into()));
        }
        let mut bytes = vec![139, 1, self.op, 0];
        bytes.extend_from_slice(&self.id.to_be_bytes());
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.payload.as_bytes());
        writer.write_all(&bytes).await?;
        Ok(())
    }
}

pub(crate) async fn read_reply<S: AsyncRead + Unpin>(
    reader: &mut S,
) -> Result<DeviceReply, VncError> {
    let version = reader.read_u8().await?;
    let op = reader.read_u8().await?;
    let status = reader.read_u8().await?;
    let id = reader.read_u32().await?;
    let len = reader.read_u32().await? as usize;
    let unsolicited = op == 0 || op == 11;
    if version != 1 || op > 11 || status > 1 || len > MAX_DEVICE_PAYLOAD
        || unsolicited != (id == 0) || (op == 11 && status != 0)
    {
        return Err(VncError::General("Invalid device reply header".into()));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    let payload = String::from_utf8(bytes)
        .map_err(|_| VncError::General("Device reply is not UTF-8".into()))?;
    Ok(DeviceReply {
        op,
        id,
        status,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_oversized_reply_before_reading_body() {
        let mut bytes = vec![1, 1, 0];
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&((MAX_DEVICE_PAYLOAD + 1) as u32).to_be_bytes());
        assert!(read_reply(&mut bytes.as_slice()).await.is_err());
    }
    #[tokio::test]
    async fn only_capabilities_and_lock_events_can_have_zero_request_id() {
        for (op, status, id, valid) in [(11, 0, 0u32, true), (11, 0, 1, false),
            (11, 1, 0, false), (7, 0, 0, false), (0, 0, 0, true)] {
            let mut bytes = vec![1, op, status];
            bytes.extend_from_slice(&id.to_be_bytes());
            bytes.extend_from_slice(&2u32.to_be_bytes());
            bytes.extend_from_slice(b"{}");
            assert_eq!(read_reply(&mut bytes.as_slice()).await.is_ok(), valid);
        }
        assert!(DeviceRequest { op: 11, id: 1, payload: "{}".into() }.write(&mut vec![]).await.is_err());
    }

    #[tokio::test]
    async fn lock_opcode_roundtrips_and_unknown_opcode_is_rejected() {
        let mut bytes = vec![];
        DeviceRequest { op: 10, id: 1, payload: "{}".into() }.write(&mut bytes).await.unwrap();
        let reply = read_reply(&mut bytes[1..].as_ref()).await.unwrap();
        assert_eq!(reply.op, 10);
        assert!(DeviceRequest { op: 11, id: 1, payload: "{}".into() }.write(&mut vec![]).await.is_err());
        bytes[2] = 12;
        assert!(read_reply(&mut bytes[1..].as_ref()).await.is_err());
    }

    #[tokio::test]
    async fn requests_and_replies_use_network_byte_order() {
        let mut bytes = vec![];
        DeviceRequest {
            op: 2,
            id: 0x12345678,
            payload: "{}".into(),
        }
        .write(&mut bytes)
        .await
        .unwrap();
        assert_eq!(
            &bytes,
            &[139, 1, 2, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 2, b'{', b'}']
        );
        let reply = read_reply(&mut bytes[1..].as_ref()).await.unwrap();
        assert_eq!(reply.id, 0x12345678);
        assert_eq!(reply.payload, "{}");
    }
}
