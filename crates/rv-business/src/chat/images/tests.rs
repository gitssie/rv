use super::super::tests::{REMOTE, detail, fixture};
use super::*;
use serde_json::json;
fn prepared() -> PreparedImage {
    let image = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 200, 0, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    prepare_bytes(bytes.get_ref()).unwrap()
}
#[test]
fn selection_preserves_aspect_ratio_and_does_not_upscale() {
    let image = image::load_from_memory(prepared().bytes()).unwrap();
    assert_eq!((image.width(), image.height()), (200, 100));
    assert!(prepare_bytes(b"not an image").is_err());
}
#[test]
fn photo_intent_preserves_draft_uses_existing_task_and_is_account_scoped() {
    let (_dir, paths, mut store, ids) = fixture();
    let plugin = super::super::vinted::VintedChat;
    let batch = plugin
        .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &detail())
        .unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let conversation = super::super::conversations(&paths).unwrap()[0].id.clone();
    crate::tasks::initialize(&paths, &ids[0]).unwrap();
    let count = store.tasks(&ids[0]).unwrap().len();
    super::super::save_draft(&paths, &conversation, "不要随图片发送的草稿").unwrap();
    let prepared = prepared();
    let queued = queue(&paths, &ids[0], &conversation, &prepared).unwrap();
    assert_eq!(
        super::super::draft(&paths, &conversation).unwrap(),
        "不要随图片发送的草稿"
    );
    assert_eq!(store.tasks(&ids[0]).unwrap().len(), count);
    let thread = super::super::thread(&paths, &conversation).unwrap();
    let image = &thread.pending[0];
    assert_eq!(image.id, queued);
    assert!(image.body.is_empty());
    assert_eq!(image.kind, "image");
    assert_eq!(image.images[0].bytes.as_deref(), Some(prepared.bytes()));
    assert!(queue(&paths, &ids[1], &conversation, &prepared).is_err());
    store
        .business_transaction(|db| {
            db.execute(
                "UPDATE chat_conversations SET allow_reply=0 WHERE id=?1",
                [&conversation],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(queue(&paths, &ids[0], &conversation, &prepared).is_err());
}
#[test]
fn uploaded_reference_and_legacy_photo_message_use_exact_platform_ids() {
    let (_dir, paths, mut store, ids) = fixture();
    let plugin = super::super::vinted::VintedChat;
    let reference = "4eedea8f-dd41-4458-8127-d04a5de14858";
    assert_eq!(
        plugin
            .image_reference(&json!({"code":0,"temp_uuid":reference}))
            .unwrap(),
        reference
    );
    assert!(plugin.image_reference(&json!({"code":0})).is_err());
    let body = plugin.reply_image(REMOTE, reference).unwrap().body.unwrap();
    assert_eq!(body["content"], "");
    assert_eq!(body["photo_temp_uuids"], json!([reference]));
    assert_eq!(body.as_object().unwrap().len(), 3);
    let mut reply = detail();
    reply["messages"] = json!([{"id":"changing-wrapper","conversation_id":REMOTE,"sender_id":"123","created_at":"2026-10-28T14:13:36Z","message_type":"legacy_reply","data":{"entity":{"id":73479112744_u64,"body":"","is_hidden":false,"photos":[{"url":"https://images.vinted.net/flower.jpg","thumbnails":[{"url":"https://images.vinted.net/tiny.jpg"}]}]}}}]);
    let batch = plugin
        .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &reply)
        .unwrap();
    assert_eq!(
        batch.messages[0].remote_id.as_deref(),
        Some("reply:73479112744")
    );
    assert_eq!(
        batch.messages[0].images[0].url,
        "https://images.vinted.net/flower.jpg"
    );
    assert_eq!(batch.conversation.preview, "[图片]");
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    reply["messages"][0]["id"] = json!("another-wrapper");
    let batch = plugin
        .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &reply)
        .unwrap();
    store
        .business_transaction(|db| repository::persist_detail(db, &ids[0], &batch))
        .unwrap();
    let conversations = super::super::conversations(&paths).unwrap();
    assert_eq!(
        super::super::thread(&paths, &conversations[0].id)
            .unwrap()
            .messages
            .len(),
        1
    );
    reply["messages"][0]["data"]["entity"]["is_hidden"] = json!(true);
    let batch = plugin
        .parse_detail(&store.load(&ids[0]).unwrap(), REMOTE, &reply)
        .unwrap();
    assert!(batch.messages[0].images.is_empty());
}
