use aletheon_abi::dasein::Stimmung;
use super::DaseinModule;
use crate::core::store::SelfFieldStore;
use rusqlite::params;

pub fn save_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();
    let mood_json = serde_json::to_string(&dasein.mood())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["mood", mood_json],
    )?;
    tracing::info!("DaseinModule state saved");
    Ok(())
}

pub fn load_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();
    let mut stmt = conn.prepare("SELECT value FROM dasein_state WHERE key = ?1")?;
    let mut rows = stmt.query(params!["mood"])?;
    if let Some(row) = rows.next()? {
        let mood_json: String = row.get(0)?;
        let mood: Stimmung = serde_json::from_str(&mood_json)?;
        *dasein.mood_raw().write() = mood;
        tracing::info!("DaseinModule mood loaded from database");
    }
    Ok(())
}
