//! New messaging gateway only. Legacy message variants remain distinct render types.
use super::*;
use rv_core::workspace::*;
use serde_json::json;
pub struct VintedChat;
fn string(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| v.as_u64().map(|n| n.to_string()))
}
fn id(v: &Value) -> Result<String, String> {
    string(v)
        .filter(|s| s.len() <= 128 && !s.chars().any(char::is_control))
        .ok_or("消息或会话 ID 无效".into())
}
// The gateway regenerates wrapper UUIDs for bridged replies on each read.
// The reply entity ID is stable and is also returned by the send endpoint.
fn message_id(v: &Value) -> Result<String, String> {
    let stable = match v["message_type"].as_str() {
        Some("legacy_reply") => v["data"]["entity"].get("id"),
        Some("reply_plain") => v["data"].get("id"),
        _ => None,
    };
    if let Some(stable) = stable {
        return id(stable).map(|id| format!("reply:{id}"));
    }
    id(&v["id"])
}
fn time(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| {
            v.as_str().and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|d| d.timestamp())
            })
        })
        .unwrap_or(0)
}
fn text(v: &Value) -> String {
    if v["data"]["is_message_content_hidden"].as_bool() == Some(true)
        || v["data"]["entity"]["is_hidden"].as_bool() == Some(true)
    {
        return "消息已隐藏".into();
    }
    match v["message_type"].as_str().unwrap_or("") {
        "text" | "text_message" | "message" | "reply_plain" => {
            v["data"]["content"].as_str().unwrap_or("").to_owned()
        }
        "legacy_portal_message" | "legacy_reply" => {
            super::notice::plain_text(v["data"]["entity"]["body"].as_str().unwrap_or(""))
        }
        "legacy_action" => v["data"]["content"]
            .as_str()
            .or_else(|| v["data"]["entity"]["body"].as_str())
            .unwrap_or("系统事件")
            .to_owned(),
        _ => v["data"]["content"]
            .as_str()
            .unwrap_or("此消息类型暂不支持展示")
            .to_owned(),
    }
}
fn image(v: &Value) -> Option<ChatImage> {
    ChatImage::from_url(v["url"].as_str().or_else(|| {
        v["thumbnails"]
            .as_array()
            .and_then(|ts| ts.iter().find_map(|t| t["url"].as_str()))
    }))
}
fn participants(v: &Value) -> Vec<Participant> {
    v["opposite_users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| matches!(u["type"].as_str(), Some("user" | "member")))
        .filter_map(|u| {
            let data = &u["data"];
            let remote_id = string(&data["id"])?;
            let name = data["login_name"]
                .as_str()
                .or_else(|| data["login"].as_str())
                .or_else(|| data["name"].as_str())
                .unwrap_or("联系人")
                .to_owned();
            Some(Participant {
                remote_id,
                username: string(&data["login_name"]).or_else(|| string(&data["login"])),
                avatar: image(&data["photo"]),
                country_code: string(&data["country_code"])
                    .or_else(|| string(&data["country"]["iso_code"]))
                    .or_else(|| string(&data["country"]["code"]))
                    .map(|s| s.to_uppercase())
                    .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase())),
                language: string(&data["language_code"]).or_else(|| {
                    string(&data["locale"])
                        .map(|s| s.split(['-', '_']).next().unwrap_or("").to_string())
                }),
                name,
                payload: crate::vinted::redact(u, 0),
            })
        })
        .collect()
}
fn conversation(app: &AppInstance, v: &Value) -> Result<Conversation, String> {
    let remote_id = id(&v["id"])?;
    uuid::Uuid::parse_str(&remote_id).map_err(|_| "新版会话必须使用 UUID")?;
    let users = participants(v);
    let title = users
        .first()
        .map(|p| p.name.clone())
        .or_else(|| {
            v["data"]["subject"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or("会话".into());
    let items = v["data"]["items"]
        .as_array()
        .or_else(|| v["data"]["inventory_items"].as_array())
        .into_iter()
        .flatten()
        .map(|i| {
            Ok(ContextItem {
                remote_id: id(i.get("inventory_item_id").unwrap_or(&i["id"]))?,
                title: i["title"].as_str().unwrap_or("商品").into(),
                price: i["price"]["amount"].as_str().map(|amount| {
                    format!(
                        "{} {}",
                        amount,
                        i["price"]["currency_code"].as_str().unwrap_or("")
                    )
                }),
                image: i["photos"]
                    .as_array()
                    .and_then(|photos| photos.first())
                    .and_then(image),
                payload: crate::vinted::redact(i, 0),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let avatar = v["opposite_users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| matches!(u["type"].as_str(), Some("user" | "member")))
        .find_map(|u| image(&u["data"]["photo"]));
    let last = &v["last_message"];
    let latest = v["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .max_by_key(|m| time(&m["created_at"]))
        .unwrap_or(last);
    let preview = text(latest);
    let has_images = [
        &latest["data"]["photos"],
        &latest["data"]["entity"]["photos"],
    ]
    .iter()
    .any(|photos| photos.as_array().is_some_and(|photos| !photos.is_empty()));
    Ok(Conversation {
        id: String::new(),
        app_id: app.id.clone(),
        remote_id,
        title,
        account: app.title.clone(),
        bundle: app.bundle_id.clone(),
        avatar,
        preview: if preview.is_empty() && has_images {
            "[图片]".into()
        } else {
            preview
        },
        updated_at: time(&latest["created_at"]).max(time(&v["created_at"])),
        unread: v["is_unread_by_current_user"].as_bool() == Some(true),
        allow_reply: match v["allow_reply"].as_bool() {
            Some(value) => Some(value && !users.is_empty()),
            None if v["opposite_users"].as_array().is_some_and(|users| {
                users.iter().any(|u| u["type"].as_str() == Some("system"))
            }) =>
            {
                Some(false)
            }
            None => None,
        },
        kind: v["conversation_type"].as_str().unwrap_or("unknown").into(),
        items,
        payload: crate::vinted::redact(v, 0),
    })
}
fn next(v: &Value) -> Result<Option<String>, String> {
    let pagination = &v["pagination"];
    match pagination["has_next"].as_bool() {
        Some(false) => Ok(None),
        Some(true) => pagination["next_cursor"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .map(|s| Some(s.to_owned()))
            .ok_or("服务端未提供下一页游标".into()),
        None => Err("消息分页字段缺失".into()),
    }
}
fn request(
    path: &str,
    cursor: Option<&str>,
    method: reqwest::Method,
    body: Option<Value>,
) -> Result<ChatRequest, String> {
    let mut url = reqwest::Url::parse("https://api.vinted.com/messaging/main/").unwrap();
    {
        let mut segments = url.path_segments_mut().map_err(|_| "消息地址无效")?;
        segments.pop_if_empty();
        for segment in path.split('/') {
            segments.push(segment);
        }
    }
    if let Some(cursor) = cursor {
        url.query_pairs_mut().append_pair("next_cursor", cursor);
    }
    Ok(ChatRequest {
        method,
        url: url.to_string(),
        body,
    })
}
impl ChatStrategy for VintedChat {
    fn requires_proxy(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            text: true,
            mark_read: true,
            upload: true,
        }
    }
    fn inbox(&self, cursor: Option<&str>) -> Result<ChatRequest, String> {
        request("inbox", cursor, reqwest::Method::GET, None)
    }
    fn detail(&self, id: &str, cursor: Option<&str>) -> Result<ChatRequest, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "会话 ID 无效")?;
        request(
            &format!("conversations/{id}"),
            cursor,
            reqwest::Method::GET,
            None,
        )
    }
    fn parse_inbox(&self, app: &AppInstance, reply: &Value) -> Result<Batch<Conversation>, String> {
        let rows = reply["conversations"]
            .as_array()
            .ok_or("会话列表结构无效")?
            .iter()
            .map(|v| conversation(app, v))
            .collect::<Result<_, _>>()?;
        Ok(Batch {
            rows,
            next: next(reply)?,
        })
    }
    fn parse_detail(
        &self,
        app: &AppInstance,
        remote: &str,
        reply: &Value,
    ) -> Result<DetailBatch, String> {
        let c = conversation(app, reply)?;
        if c.remote_id != remote {
            return Err("回复的会话与请求不一致".into());
        }
        let metadata: Value = serde_json::from_str(&app.metadata).unwrap_or_default();
        let own = string(&metadata["user_id"]);
        let mut messages = Vec::new();
        for m in reply["messages"].as_array().ok_or("消息列表结构无效")? {
            if m["conversation_id"].as_str() != Some(remote) {
                return Err("消息会话归属不一致".into());
            }
            let remote_id = message_id(m)?;
            let sender = string(&m["sender_id"]).unwrap_or_default();
            let mut images = Vec::new();
            if m["data"]["is_message_content_hidden"].as_bool() != Some(true)
                && m["data"]["entity"]["is_hidden"].as_bool() != Some(true)
            {
                for photos in [&m["data"]["photos"], &m["data"]["entity"]["photos"]] {
                    images.extend(photos.as_array().into_iter().flatten().filter_map(image));
                }
            }
            messages.push(Message {
                translation: None,
                id: String::new(),
                remote_id: Some(remote_id),
                outgoing: own.as_deref() == Some(sender.as_str()),
                sender_id: sender,
                created_at: time(&m["created_at"]),
                kind: m["message_type"].as_str().unwrap_or("unknown").into(),
                body: text(m),
                status: "received".into(),
                images,
                payload: crate::vinted::redact(m, 0),
            });
        }
        Ok(DetailBatch {
            conversation: c,
            messages,
            participants: participants(reply),
            next: next(reply)?,
        })
    }
    fn reply(&self, id: &str, text: &str) -> Result<ChatRequest, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "会话 ID 无效")?;
        if text.trim().is_empty() || text.len() > 65536 {
            return Err("消息为空或过长".into());
        }
        request(
            &format!("conversations/{id}/replies"),
            None,
            reqwest::Method::POST,
            Some(json!({"content":text,"is_personal_data_sharing_check_skipped":false})),
        )
    }
    fn sent_message_id(&self, conversation: &str, reply: &Value) -> Option<String> {
        let message = reply.get("message").unwrap_or(reply);
        if message["conversation_id"]
            .as_str()
            .is_some_and(|id| id != conversation)
        {
            return None;
        }
        message_id(message).ok()
    }
    fn image_upload(&self) -> Result<ImageUpload, String> {
        Ok(ImageUpload {
            url: "https://www.vinted.com/api/v2/photos".into(),
            fields: vec![("photo[type]".into(), "user_msg".into())],
            file_field: "photo[file]".into(),
        })
    }
    fn image_reference(&self, reply: &Value) -> Result<String, String> {
        if reply.get("code").is_some_and(|code| code != 0) {
            return Err("图片上传未成功".into());
        }
        reply["temp_uuid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "图片上传回执缺少引用，结果待确认".into())
    }
    fn reply_image(&self, id: &str, reference: &str) -> Result<ChatRequest, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "会话 ID 无效")?;
        uuid::Uuid::parse_str(reference).map_err(|_| "图片引用无效")?;
        request(
            &format!("conversations/{id}/replies"),
            None,
            reqwest::Method::POST,
            Some(
                json!({"content":"","photo_temp_uuids":[reference],"is_personal_data_sharing_check_skipped":false}),
            ),
        )
    }
    fn mark_read(&self, id: &str) -> Result<ChatRequest, String> {
        uuid::Uuid::parse_str(id).map_err(|_| "会话 ID 无效")?;
        request(
            &format!("conversations/{id}/mark_as_read"),
            None,
            reqwest::Method::POST,
            None,
        )
    }
}
