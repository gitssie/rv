//! Isolated SQLite fixtures; no real account, proxy or phone is used.
use rv_business::{chat::*, shop::ShopProfile};
use rv_core::{ConnectionId, StorePaths, workspace::*};
use serde_json::json;
pub(in crate::app) fn seed(paths: &StorePaths, device: ConnectionId) -> Vec<AppId> {
    let ids = vec![
        AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap(),
        AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F4").unwrap(),
        AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F5").unwrap(),
    ];
    let mut store = WorkspaceStore::open(paths).unwrap();
    let catalog = json!({"bundle_id":"lt.manodrabuziai.fr","backups":ids.iter().map(|id|json!({"kind":"snapshot","idfv":id.as_str(),"backup_id":format!("snapshot:{}",id.as_str()),"display_name":"fixture backup"})).collect::<Vec<_>>()});
    store
        .sync_backups(device, "lt.manodrabuziai.fr", &catalog)
        .unwrap();
    for (i, id) in ids.iter().enumerate() {
        let mut app = store.load(id).unwrap();
        app.metadata=json!({"shop":ShopProfile{user_id:i as u64+1,login:Some(["thomasl5400","marine_paris","alice_shop"][i].into()),country_code:Some("FR".into()),..Default::default()}}).to_string();
        store.save(&app).unwrap();
        store
            .save_auth(
                id,
                &AppAuth {
                    access_token: "fixture-never-used-for-http".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        rv_business::tasks::initialize(paths, id).unwrap();
    }
    conversations(paths).unwrap();
    let now = 1_793_095_000;
    for (index, (app, title)) in [(0, "carmen_m"), (0, "camille_d"), (1, "julie.m")]
        .iter()
        .enumerate()
    {
        let c = Conversation {
            id: format!("conversation-{index}"),
            app_id: ids[*app].clone(),
            remote_id: uuid::Uuid::from_u128(index as u128 + 1).to_string(),
            title: (*title).into(),
            account: String::new(),
            bundle: "lt.manodrabuziai.fr".into(),
            avatar: None,
            preview: "Hola, ¿sigue disponible?".into(),
            updated_at: now - index as i64,
            unread: true,
            allow_reply: Some(true),
            kind: "private".into(),
            items: vec![ContextItem {
                remote_id: "item-1".into(),
                title: "Pull en laine bleu marine".into(),
                price: Some("18,00 €".into()),
                image: None,
                payload: json!({}),
            }],
            payload: json!({}),
        };
        store.business_transaction(|db|{
   db.execute("INSERT INTO chat_conversations(id,app_id,remote_id,title,preview,updated_at,unread,allow_reply,detail_loaded,data) VALUES(?1,?2,?3,?4,?5,?6,1,1,1,?7)",rusqlite::params![c.id,c.app_id.as_str(),c.remote_id,c.title,c.preview,c.updated_at,serde_json::to_string(&c)?])?;
   let customer=format!("customer-{index}");
   db.execute("INSERT INTO chat_customers(id,app_id,remote_user_id,username,display_name,country_code,preferred_language,language_source,data,created_at,updated_at) VALUES(?1,?2,?3,?4,?4,'ES','es','country','{}',?5,?5)",rusqlite::params![customer,c.app_id.as_str(),format!("user-{index}"),c.title,now])?;
   db.execute("INSERT INTO chat_conversation_customers(conversation_id,customer_id) VALUES(?1,?2)",rusqlite::params![c.id,customer])?;
   for (n,(outgoing,body)) in [(false,"Hola, ¿sigue disponible?"),(true,"你好，还在售。"),(false,"¿Me puedes confirmar la talla?")].iter().enumerate(){
    let m=Message{id:format!("message-{index}-{n}"),remote_id:Some(format!("remote-{index}-{n}")),outgoing:*outgoing,sender_id:if *outgoing{"shop"}else{"customer"}.into(),created_at:now-60+n as i64,status:"sent".into(),kind:"text".into(),body:(*body).into(),images:vec![],translation:if *outgoing{None}else{Some(translation::Translation{original:(*body).into(),translated:if n==0{"你好，还在售吗？"}else{"可以确认一下尺码吗？"}.into(),source_language:Some("es".into()),target_language:"zh".into()})},payload:json!({})};
    db.execute("INSERT INTO chat_messages(id,conversation_id,remote_id,created_at,body,data) VALUES(?1,?2,?3,?4,?5,?6)",rusqlite::params![m.id,c.id,m.remote_id,m.created_at,m.body,serde_json::to_string(&m)?])?;
   }
   Ok(())
  }).unwrap();
    }
    ids
}
