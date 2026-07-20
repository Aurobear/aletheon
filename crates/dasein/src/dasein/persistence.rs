use super::DaseinModule;
use crate::core::store::SelfFieldStore;
use fabric::dasein::{ExperienceSource, InterpretedExperience, Stimmung};
use rusqlite::{params, OptionalExtension};

pub fn save_dasein_state(dasein: &DaseinModule, _store: &SelfFieldStore) -> anyhow::Result<()> {
    dasein.checkpoint_durable_state()
}

pub async fn load_dasein_state(
    dasein: &DaseinModule,
    store: &SelfFieldStore,
) -> anyhow::Result<()> {
    let replayed = dasein.replay_durable_state().await?;
    if replayed == 0 {
        migrate_legacy_mood(dasein, store).await?;
    } else {
        dasein.record_resumption_after_replay().await?;
    }
    Ok(())
}

async fn migrate_legacy_mood(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let mood_json = store
        .conn()
        .query_row(
            "SELECT value FROM dasein_state WHERE key = ?1",
            params!["mood"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(mood_json) = mood_json else {
        return Ok(());
    };
    let mood: Stimmung = serde_json::from_str(&mood_json)?;
    dasein
        .transition_current_for_restore(
            ExperienceSource::Dasein,
            "legacy-dasein-state-migration",
            InterpretedExperience::MoodObserved {
                mood,
                reason: "migrated legacy mood record".into(),
            },
        )
        .await?;
    Ok(())
}
