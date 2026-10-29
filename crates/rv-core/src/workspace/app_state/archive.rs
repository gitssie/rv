use super::*;
use std::io::{Cursor, Read};

pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub(super) const NAMES: [&str; 3] = [
    "keychain/records.plist",
    "identity/state.plist",
    "preferences/records.plist",
];

pub fn state_sha256(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn parse(data: &[u8]) -> Result<Plist, StoreError> {
    Plist::from_reader(Cursor::new(data)).map_err(|_| invalid("state plist 格式错误"))
}
pub(super) fn encode(value: &Plist) -> Result<Vec<u8>, StoreError> {
    let mut data = Vec::new();
    value
        .to_writer_binary(&mut data)
        .map_err(|_| invalid("无法编码 state plist"))?;
    Ok(data)
}
pub(super) fn text<'a>(value: &'a Plist, key: &str) -> Option<&'a str> {
    value.as_dictionary()?.get(key)?.as_string()
}
pub(super) fn at<'a>(value: &'a Plist, key: &str) -> Result<&'a Plist, StoreError> {
    value
        .as_dictionary()
        .and_then(|d| d.get(key))
        .ok_or_else(|| invalid("state 缺少必要字段"))
}
pub(super) fn integer(value: &Plist, key: &str) -> Result<u64, StoreError> {
    at(value, key)?
        .as_unsigned_integer()
        .ok_or_else(|| invalid("state 整数字段无效"))
}
pub(super) fn set(value: &mut Plist, key: &str, data: impl Into<Plist>) -> Result<(), StoreError> {
    value
        .as_dictionary_mut()
        .ok_or_else(|| invalid("state 字典无效"))?
        .insert(key.into(), data.into());
    Ok(())
}
pub(super) fn unpack(
    manifest: &[u8],
    archive: &[u8],
    id: &AppId,
    bundle: &str,
) -> Result<[Vec<u8>; 3], StoreError> {
    if manifest.len() > MAX_MANIFEST_BYTES || archive.len() > MAX_STATE_BYTES {
        return Err(invalid("state 超过大小限制"));
    }
    let manifest = parse(manifest)?;
    if text(&manifest, "format") != Some("ShadowAppState2")
        || text(&manifest, "bundleID") != Some(bundle)
        || text(&manifest, "idfv") != Some(id.as_str())
    {
        return Err(invalid("Shadow state 格式或身份不匹配"));
    }
    let info = at(&manifest, "stateArchiveInfo")?;
    if text(info, "format") != Some("pax")
        || text(info, "compression") != Some("zstd")
        || integer(info, "archiveSize")? != archive.len() as u64
        || text(info, "sha256") != Some(&state_sha256(archive))
    {
        return Err(invalid("state 与清单不匹配"));
    }
    let entries = at(info, "entries")?
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| invalid("state 文件清单无效"))?;
    let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(archive))
        .map_err(|_| invalid("state zstd 格式无效"))?;
    decoder
        .window_log_max(26)
        .map_err(|_| invalid("state zstd 窗口无效"))?;
    let mut tar = tar::Archive::new(decoder.take((MAX_STATE_BYTES + 1) as u64));
    let mut result: [Vec<u8>; 3] = Default::default();
    let mut seen = [false; 3];
    for entry in tar.entries().map_err(|_| invalid("state tar 格式无效"))? {
        let mut entry = entry.map_err(|_| invalid("state tar 文件无效"))?;
        let path = entry.path().map_err(|_| invalid("state 文件路径无效"))?;
        let index = NAMES
            .iter()
            .position(|name| path == std::path::Path::new(name))
            .ok_or_else(|| invalid("state 包含未知文件"))?;
        if seen[index]
            || !entry.header().entry_type().is_file()
            || entry.size() > MAX_PAYLOAD_BYTES as u64
        {
            return Err(invalid("state 文件重复、过大或类型无效"));
        }
        let expected = entries
            .iter()
            .find(|e| text(e, "path") == Some(NAMES[index]))
            .ok_or_else(|| invalid("state 清单缺少文件"))?;
        entry
            .read_to_end(&mut result[index])
            .map_err(|_| invalid("state 文件读取失败"))?;
        if integer(expected, "size")? != result[index].len() as u64
            || text(expected, "sha256") != Some(&state_sha256(&result[index]))
        {
            return Err(invalid("state 内容与清单不匹配"));
        }
        parse(&result[index])?;
        seen[index] = true;
    }
    if !seen.into_iter().all(|v| v) {
        return Err(invalid("state 文件不完整"));
    }
    let identity = parse(&result[1])?;
    if text(at(&identity, "data")?, "VendorIdentifier") != Some(id.as_str()) {
        return Err(invalid("state 指纹与 IDFV 不匹配"));
    }
    Ok(result)
}

/// Rebuild only the state archive. App data archive information and all unknown
/// manifest/plist fields survive. No live phone/container writes occur here.
pub(super) fn repack(
    manifest: &[u8],
    payloads: &[Vec<u8>; 3],
) -> Result<(Vec<u8>, Vec<u8>), StoreError> {
    let mut manifest = parse(manifest)?;
    let info = manifest
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("stateArchiveInfo"))
        .ok_or_else(|| invalid("state 清单无效"))?;
    let entries = info
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("entries"))
        .and_then(Plist::as_array_mut)
        .ok_or_else(|| invalid("state 文件清单无效"))?;
    let encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)
        .map_err(|_| invalid("无法创建 state 压缩器"))?;
    let mut tar = tar::Builder::new(encoder);
    for (index, name) in NAMES.iter().enumerate() {
        let entry = entries
            .iter_mut()
            .find(|e| text(e, "path") == Some(*name))
            .ok_or_else(|| invalid("state 文件清单不完整"))?;
        let data = &payloads[index];
        if data.len() > MAX_PAYLOAD_BYTES {
            return Err(invalid("state plist 过大"));
        }
        let birth = format!(
            "{}:{:09}",
            integer(entry, "birthtime")?,
            integer(entry, "birthtimeNsec")?
        );
        let mtime = format!(
            "{}.{:09}",
            integer(entry, "mtime")?,
            integer(entry, "mtimeNsec")?
        );
        let mut pax = vec![("SCHILY.xattr.org.shadow.birthtime", birth.as_bytes())];
        if integer(entry, "mtimeNsec")? > 0 {
            pax.push(("mtime", mtime.as_bytes()));
        }
        tar.append_pax_extensions(pax)
            .map_err(|_| invalid("无法编码 state PAX 元数据"))?;
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(integer(entry, "mode")? as u32);
        header.set_uid(integer(entry, "uid")?);
        header.set_gid(integer(entry, "gid")?);
        header.set_mtime(integer(entry, "mtime")?);
        header.set_size(data.len() as u64);
        header.set_cksum();
        tar.append_data(&mut header, *name, data.as_slice())
            .map_err(|_| invalid("无法编码 state 文件"))?;
        set(entry, "size", data.len() as u64)?;
        set(entry, "sha256", state_sha256(data))?;
    }
    let encoder = tar
        .into_inner()
        .map_err(|_| invalid("无法完成 state tar"))?;
    let archive = encoder
        .finish()
        .map_err(|_| invalid("无法完成 state 压缩"))?;
    if archive.len() > MAX_STATE_BYTES {
        return Err(invalid("state 归档过大"));
    }
    set(info, "sha256", state_sha256(&archive))?;
    set(info, "archiveSize", archive.len() as u64)?;
    Ok((encode(&manifest)?, archive))
}
