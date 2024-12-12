use bioma_actor::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use surrealdb::RecordId;
use test_log::test;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DummyMessage {
    /// Message id created with a ULID
    id: RecordId,
    /// Message name (usually a type name)
    pub name: Cow<'static, str>,
    /// Sender
    pub tx: RecordId,
    /// Receiver
    pub rx: RecordId,
    /// Message content
    pub msg: Value,
}

#[test(tokio::test)]
async fn test_engine_db_write() -> Result<(), SystemActorError> {
    let engine = Engine::test().await?;

    let db = engine.db();

    let msg = DummyMessage {
        id: RecordId::from_table_key("test_engine_db_write", "0000001"),
        name: Cow::Borrowed("test_engine_db_write"),
        tx: RecordId::from_table_key("test_engine_db_write", "0000002"),
        rx: RecordId::from_table_key("test_engine_db_write", "0000003"),
        msg: Value::Null,
    };

    let record: Option<Record> = db.lock().await.create("test_engine_db_write").content(msg).await?;

    assert_eq!(record.unwrap().id, RecordId::from_table_key("test_engine_db_write", "0000001"));

    dbg_export_db!(engine);

    Ok(())
}
