use std::io::Cursor;

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};

use crate::{ExtendedClipboardEvent, PixelFormat, Rect, VncEncoding, VncError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CLIPBOARD_TEXT: u32 = 1;
const CLIPBOARD_CAPS: u32 = 1 << 24;
const CLIPBOARD_REQUEST: u32 = 1 << 25;
const CLIPBOARD_PEEK: u32 = 1 << 26;
const CLIPBOARD_NOTIFY: u32 = 1 << 27;
const CLIPBOARD_PROVIDE: u32 = 1 << 28;
const CLIPBOARD_ACTIONS: u32 = 0xFF00_0000;
const CLIPBOARD_FORMATS: u32 = 0x0000_FFFF;
const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) enum ClientMsg {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<VncEncoding>),
    FramebufferUpdateRequest(Rect, u8),
    KeyEvent(u32, bool),
    PointerEvent(u16, u16, u8),
    ClientCutText(String),
    ExtendedClipboardCaps,
    ExtendedClipboardNotify(bool),
    ExtendedClipboardRequest,
    ExtendedClipboardProvide(String),
}

impl ClientMsg {
    pub(super) async fn write<S>(self, writer: &mut S) -> Result<(), VncError>
    where
        S: AsyncWrite + Unpin,
    {
        match self {
            ClientMsg::SetPixelFormat(pf) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [0]       | message-type |
                // | 3            |              | padding      |
                // | 16           | PIXEL_FORMAT | pixel-format |
                // +--------------+--------------+--------------+
                let mut payload = vec![0_u8, 0, 0, 0];
                payload.extend(<PixelFormat as Into<Vec<u8>>>::into(pf));
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::SetEncodings(encodings) => {
                //  +--------------+--------------+---------------------+
                // | No. of bytes | Type [Value] | Description         |
                // +--------------+--------------+---------------------+
                // | 1            | U8 [2]       | message-type        |
                // | 1            |              | padding             |
                // | 2            | U16          | number-of-encodings |
                // +--------------+--------------+---------------------+

                // This is followed by number-of-encodings repetitions of the following:
                // +--------------+--------------+---------------+
                // | No. of bytes | Type [Value] | Description   |
                // +--------------+--------------+---------------+
                // | 4            | S32          | encoding-type |
                // +--------------+--------------+---------------+
                let mut payload = vec![2, 0];
                payload.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
                for e in encodings {
                    payload.write_u32(e.into()).await?;
                }
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::FramebufferUpdateRequest(rect, incremental) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 1            | U8           | incremental  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // | 2            | U16          | width        |
                // | 2            | U16          | height       |
                // +--------------+--------------+--------------+
                let mut payload = vec![3, incremental];
                payload.extend_from_slice(&rect.x.to_be_bytes());
                payload.extend_from_slice(&rect.y.to_be_bytes());
                payload.extend_from_slice(&rect.width.to_be_bytes());
                payload.extend_from_slice(&rect.height.to_be_bytes());
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::KeyEvent(keycode, down) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [4]       | message-type |
                // | 1            | U8           | down-flag    |
                // | 2            |              | padding      |
                // | 4            | U32          | key          |
                // +--------------+--------------+--------------+
                let mut payload = vec![4, down as u8, 0, 0];
                payload.write_u32(keycode).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::PointerEvent(x, y, mask) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [5]       | message-type |
                // | 1            | U8           | button-mask  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // +--------------+--------------+--------------+
                let mut payload = vec![5, mask];
                payload.write_u16(x).await?;
                payload.write_u16(y).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::ClientCutText(s) => {
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [6]       | message-type |
                //   | 3            |              | padding      |
                //   | 4            | U32          | length       |
                //   | length       | U8 array     | text         |
                //   +--------------+--------------+--------------+
                let text = encode_latin1(&s)?;
                let mut payload = vec![6_u8, 0, 0, 0];
                payload.write_u32(text.len() as u32).await?;
                payload.write_all(&text).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::ExtendedClipboardCaps => {
                let flags = CLIPBOARD_CAPS
                    | CLIPBOARD_REQUEST
                    | CLIPBOARD_PEEK
                    | CLIPBOARD_NOTIFY
                    | CLIPBOARD_PROVIDE
                    | CLIPBOARD_TEXT;
                let mut data = flags.to_be_bytes().to_vec();
                data.extend_from_slice(&0_u32.to_be_bytes());
                write_extended_clipboard(writer, &data).await
            }
            ClientMsg::ExtendedClipboardNotify(has_text) => {
                let flags = CLIPBOARD_NOTIFY | (u32::from(has_text) * CLIPBOARD_TEXT);
                write_extended_clipboard(writer, &flags.to_be_bytes()).await
            }
            ClientMsg::ExtendedClipboardRequest => {
                let flags = CLIPBOARD_REQUEST | CLIPBOARD_TEXT;
                write_extended_clipboard(writer, &flags.to_be_bytes()).await
            }
            ClientMsg::ExtendedClipboardProvide(text) => {
                if text.len() >= MAX_CLIPBOARD_BYTES {
                    return Err(VncError::General("clipboard text is too large".to_string()));
                }
                let normalized = text.replace("\r\n", "\n").replace(['\r', '\n'], "\r\n");
                let mut plain = Vec::with_capacity(normalized.len() + 5);
                let text_len = normalized
                    .len()
                    .checked_add(1)
                    .ok_or_else(|| VncError::General("clipboard text is too large".to_string()))?;
                if text_len + 4 > MAX_CLIPBOARD_BYTES {
                    return Err(VncError::General("clipboard text is too large".to_string()));
                }
                plain.extend_from_slice(&(text_len as u32).to_be_bytes());
                plain.extend_from_slice(normalized.as_bytes());
                plain.push(0);
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                std::io::Write::write_all(&mut encoder, &plain)?;
                let compressed = encoder.finish()?;
                let mut data = (CLIPBOARD_PROVIDE | CLIPBOARD_TEXT).to_be_bytes().to_vec();
                data.extend_from_slice(&compressed);
                write_extended_clipboard(writer, &data).await
            }
        }
    }
}

fn encode_latin1(text: &str) -> Result<Vec<u8>, VncError> {
    text.chars()
        .map(|character| {
            u8::try_from(character as u32)
                .map_err(|_| VncError::General("clipboard text is not valid Latin-1".to_string()))
        })
        .collect()
}

async fn write_extended_clipboard<S>(writer: &mut S, data: &[u8]) -> Result<(), VncError>
where
    S: AsyncWrite + Unpin,
{
    if data.len() > MAX_CLIPBOARD_BYTES {
        return Err(VncError::General(
            "clipboard payload is too large".to_string(),
        ));
    }
    let len = i32::try_from(data.len())
        .map_err(|_| VncError::General("clipboard payload is too large".to_string()))?;
    let mut payload = vec![6_u8, 0, 0, 0];
    payload.extend_from_slice(&(-len).to_be_bytes());
    payload.extend_from_slice(data);
    writer.write_all(&payload).await?;
    Ok(())
}

#[derive(Debug)]
pub(super) enum ServerMsg {
    FramebufferUpdate(u16),
    // SetColorMapEntries,
    Bell,
    ServerCutText(String),
    ExtendedClipboard(ExtendedClipboardEvent),
}

impl ServerMsg {
    pub(super) async fn read<S>(reader: &mut S) -> Result<Self, VncError>
    where
        S: AsyncRead + Unpin,
    {
        let server_msg = reader.read_u8().await?;

        match server_msg {
            0 => {
                // FramebufferUpdate
                //   +--------------+--------------+----------------------+
                //   | No. of bytes | Type [Value] | Description          |
                //   +--------------+--------------+----------------------+
                //   | 1            | U8 [0]       | message-type         |
                //   | 1            |              | padding              |
                //   | 2            | U16          | number-of-rectangles |
                //   +--------------+--------------+----------------------+
                let _padding = reader.read_u8().await?;
                let rects = reader.read_u16().await?;
                Ok(ServerMsg::FramebufferUpdate(rects))
            }
            1 => {
                // SetColorMapEntries
                // +--------------+--------------+------------------+
                // | No. of bytes | Type [Value] | Description      |
                // +--------------+--------------+------------------+
                // | 1            | U8 [1]       | message-type     |
                // | 1            |              | padding          |
                // | 2            | U16          | first-color      |
                // | 2            | U16          | number-of-colors |
                // +--------------+--------------+------------------+
                unimplemented!()
            }
            2 => {
                // Bell
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [2]       | message-type |
                //   +--------------+--------------+--------------+
                Ok(ServerMsg::Bell)
            }
            3 => {
                // ServerCutText
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 3            |              | padding      |
                // | 4            | U32          | length       |
                // | length       | U8 array     | text         |
                // +--------------+--------------+--------------+
                let mut padding = [0; 3];
                reader.read_exact(&mut padding).await?;
                let len = reader.read_i32().await?;
                if len < 0 {
                    let len = len
                        .checked_neg()
                        .ok_or_else(|| VncError::General("invalid clipboard length".to_string()))?
                        as usize;
                    if len > MAX_CLIPBOARD_BYTES {
                        return Err(VncError::General(
                            "extended clipboard payload is too large".to_string(),
                        ));
                    }
                    let mut data = vec![0; len];
                    reader.read_exact(&mut data).await?;
                    Ok(Self::ExtendedClipboard(decode_extended_clipboard(&data)?))
                } else {
                    let len = len as usize;
                    if len > MAX_CLIPBOARD_BYTES {
                        return Err(VncError::General(
                            "clipboard payload is too large".to_string(),
                        ));
                    }
                    let mut bytes = vec![0; len];
                    reader.read_exact(&mut bytes).await?;
                    let text: String = bytes.into_iter().map(char::from).collect();
                    Ok(Self::ServerCutText(text))
                }
            }
            _ => Err(VncError::WrongServerMessage),
        }
    }
}

fn decode_extended_clipboard(data: &[u8]) -> Result<ExtendedClipboardEvent, VncError> {
    if data.len() < 4 {
        return Err(VncError::General(
            "extended clipboard payload is truncated".to_string(),
        ));
    }
    let flags = u32::from_be_bytes(data[..4].try_into().unwrap());
    let actions = flags & CLIPBOARD_ACTIONS;
    let formats = flags & CLIPBOARD_FORMATS;
    if actions & CLIPBOARD_CAPS != 0 {
        let required = formats.count_ones() as usize * 4;
        if data.len() < 4 + required {
            return Err(VncError::General(
                "extended clipboard capabilities are truncated".to_string(),
            ));
        }
        let mut format_sizes = [0; 16];
        let mut offset = 4;
        for (bit, size) in format_sizes.iter_mut().enumerate() {
            if formats & (1 << bit) == 0 {
                continue;
            }
            *size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
        }
        return Ok(ExtendedClipboardEvent::Caps {
            flags,
            format_sizes,
        });
    }
    match actions {
        CLIPBOARD_REQUEST => Ok(ExtendedClipboardEvent::Request(formats)),
        CLIPBOARD_PEEK => Ok(ExtendedClipboardEvent::Peek),
        CLIPBOARD_NOTIFY => Ok(ExtendedClipboardEvent::Notify(formats)),
        CLIPBOARD_PROVIDE => {
            decode_clipboard_provide(formats, &data[4..]).map(ExtendedClipboardEvent::Provide)
        }
        _ => Err(VncError::General(
            "unsupported extended clipboard action".to_string(),
        )),
    }
}

fn decode_clipboard_provide(formats: u32, compressed: &[u8]) -> Result<String, VncError> {
    let mut decoder = std::io::Read::take(
        ZlibDecoder::new(compressed),
        (MAX_CLIPBOARD_BYTES + 1) as u64,
    );
    let mut plain = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut plain)?;
    if plain.len() > MAX_CLIPBOARD_BYTES {
        return Err(VncError::General(
            "decompressed clipboard payload is too large".to_string(),
        ));
    }
    let mut cursor = Cursor::new(plain);
    let mut text = None;
    for bit in 0..16 {
        if formats & (1 << bit) == 0 {
            continue;
        }
        let mut len = [0; 4];
        std::io::Read::read_exact(&mut cursor, &mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_CLIPBOARD_BYTES {
            return Err(VncError::General(
                "clipboard format is too large".to_string(),
            ));
        }
        let mut value = vec![0; len];
        std::io::Read::read_exact(&mut cursor, &mut value)?;
        if bit == 0 {
            if value.last() == Some(&0) {
                value.pop();
            }
            let decoded = String::from_utf8(value).map_err(|_| {
                VncError::General("clipboard payload is not valid UTF-8".to_string())
            })?;
            text = Some(decoded.replace("\r\n", "\n"));
        }
    }
    text.ok_or_else(|| VncError::General("clipboard text format is missing".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wire_for(message: ClientMsg) -> Vec<u8> {
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        message.write(&mut writer).await.unwrap();
        drop(writer);
        let mut wire = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut wire)
            .await
            .unwrap();
        wire
    }

    #[tokio::test]
    async fn legacy_clipboard_writes_latin1_bytes() {
        let wire = wire_for(ClientMsg::ClientCutText("café".into())).await;
        assert_eq!(&wire[..8], &[6, 0, 0, 0, 0, 0, 0, 4]);
        assert_eq!(&wire[8..], b"caf\xe9");
    }

    #[tokio::test]
    async fn extended_clipboard_writes_negative_length_and_utf8_text() {
        let wire = wire_for(ClientMsg::ExtendedClipboardProvide("中文\n第二行".into())).await;
        assert_eq!(&wire[..4], &[6, 0, 0, 0]);
        let length = i32::from_be_bytes(wire[4..8].try_into().unwrap());
        assert_eq!((-length) as usize, wire.len() - 8);
        assert!(matches!(
            decode_extended_clipboard(&wire[8..]).unwrap(),
            ExtendedClipboardEvent::Provide(text) if text == "中文\n第二行"
        ));
    }

    #[tokio::test]
    async fn server_cut_text_decodes_latin1_instead_of_lossy_utf8() {
        let mut wire: &[u8] = &[3, 0, 0, 0, 0, 0, 0, 4, b'c', b'a', b'f', 0xe9];
        assert!(matches!(
            ServerMsg::read(&mut wire).await.unwrap(),
            ServerMsg::ServerCutText(text) if text == "café"
        ));
    }

    #[test]
    fn trollvnc_capabilities_preserve_unsolicited_text_limit() {
        let data = [0x17, 0, 0, 1, 0, 0x10, 0, 0];
        assert!(matches!(
            decode_extended_clipboard(&data).unwrap(),
            ExtendedClipboardEvent::Caps {
                flags: 0x1700_0001,
                format_sizes,
            } if format_sizes[0] == 0x0010_0000
        ));
    }
}
