//! Counterparty data and language choices, independent of chat rendering and HTTP.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use rv_core::{StoreError, StorePaths, workspace::*};
pub(super) fn persist(
    db: &Connection,
    app: &AppId,
    conversation: &str,
    people: &[Participant],
) -> Result<(), StoreError> {
    db.execute(
        "DELETE FROM chat_conversation_customers WHERE conversation_id=?1",
        [conversation],
    )?;
    for p in people {
        let country = p.country_code.clone();
        let language = p.language.clone();
        let source = if language.is_some() {
            Some("profile")
        } else if country
            .as_deref()
            .and_then(translation::country_language)
            .is_some()
        {
            Some("country")
        } else {
            None
        };
        let language = language.or_else(|| {
            country
                .as_deref()
                .and_then(translation::country_language)
                .map(str::to_owned)
        });
        let avatar = p.avatar.as_ref().map(|image| image.url.clone());
        let timestamp = super::service::now();
        db.execute("INSERT INTO chat_customers(id,app_id,remote_user_id,username,display_name,avatar_url,country_code,preferred_language,language_source,data,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11) ON CONFLICT(app_id,remote_user_id) DO UPDATE SET username=excluded.username,display_name=excluded.display_name,avatar_url=COALESCE(excluded.avatar_url,chat_customers.avatar_url),country_code=COALESCE(excluded.country_code,chat_customers.country_code),preferred_language=CASE WHEN chat_customers.language_source='manual' THEN chat_customers.preferred_language ELSE COALESCE(excluded.preferred_language,chat_customers.preferred_language) END,language_source=CASE WHEN chat_customers.language_source='manual' THEN 'manual' ELSE COALESCE(excluded.language_source,chat_customers.language_source) END,data=excluded.data,updated_at=excluded.updated_at",params![uuid::Uuid::new_v4().to_string(),app.as_str(),p.remote_id,p.username.clone().unwrap_or_else(||p.name.clone()),p.name,avatar,country,language,source,p.payload.to_string(),timestamp])?;
        db.execute("INSERT OR IGNORE INTO chat_conversation_customers(conversation_id,customer_id) SELECT ?1,id FROM chat_customers WHERE app_id=?2 AND remote_user_id=?3",params![conversation,app.as_str(),p.remote_id])?;
    }
    Ok(())
}
pub(super) fn for_conversation(db: &Connection, id: &str) -> Result<Option<Customer>, StoreError> {
    db.query_row("SELECT c.id,c.display_name,c.country_code,c.preferred_language,c.language_source,c.tags,c.note FROM chat_customers c JOIN chat_conversation_customers link ON link.customer_id=c.id WHERE link.conversation_id=?1 ORDER BY c.id LIMIT 1",[id],|r|Ok(Customer{id:r.get(0)?,name:r.get(1)?,country_code:r.get(2)?,language:r.get(3)?,language_source:r.get(4)?,tags:serde_json::from_str(&r.get::<_,String>(5)?).unwrap_or_default(),note:r.get(6)?})).optional().map_err(Into::into)
}
pub fn set_customer_language(
    paths: &StorePaths,
    conversation: &str,
    language: &str,
) -> Result<(), String> {
    if !["zh", "en", "fr", "es", "de", "it", "pt", "nl"].contains(&language) {
        return Err("语言不受支持".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store.business_transaction(|db|{super::repository::schema(db)?;let customer=for_conversation(db,conversation)?.ok_or_else(||StoreError::Invalid("尚未读取客户资料".into()))?;db.execute("UPDATE chat_customers SET preferred_language=?2,language_source='manual',updated_at=?3 WHERE id=?1",params![customer.id,language,super::service::now()])?;Ok(())}).map_err(|e|e.to_string())
}
