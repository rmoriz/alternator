use crate::error::{AlternatorError, MastodonError};
use crate::mastodon::{MastodonStream, MediaRecreation};
use tracing::debug;

/// Recreate media attachments with descriptions and race condition checks
pub async fn recreate_media_with_race_check(
    mastodon_client: &impl MastodonStream,
    toot_id: &str,
    media_recreations: Vec<MediaRecreation>, // Vec of media recreations with descriptions
    original_media_ids: Vec<String>,         // Original media IDs to clean up after success
) -> Result<(), AlternatorError> {
    if media_recreations.is_empty() {
        return Ok(());
    }

    // Get current toot state to verify no race conditions
    let current_toot = mastodon_client
        .get_toot(toot_id)
        .await
        .map_err(AlternatorError::Mastodon)?;

    // Check that all media we're trying to recreate still exist and need descriptions
    // media_recreations and original_media_ids are parallel arrays
    for media_id in &original_media_ids {
        if let Some(current_media) = current_toot
            .media_attachments
            .iter()
            .find(|m| m.id == *media_id)
        {
            // Check if this media already has a description (processed by another instance)
            if current_media.description.is_some()
                && !current_media
                    .description
                    .as_ref()
                    .unwrap()
                    .trim()
                    .is_empty()
            {
                debug!(
                    "Media {} already has description, race condition detected",
                    media_id
                );
                return Err(AlternatorError::Mastodon(
                    MastodonError::RaceConditionDetected,
                ));
            }
        } else {
            // Media attachment not found in current toot state
            debug!(
                "Media {} no longer exists in toot {}, race condition detected",
                media_id, toot_id
            );
            return Err(AlternatorError::Mastodon(
                MastodonError::RaceConditionDetected,
            ));
        }
    }

    // Recreate all media attachments with descriptions (includes cleanup)
    match mastodon_client
        .recreate_media_with_descriptions(toot_id, media_recreations, original_media_ids)
        .await
    {
        Ok(()) => Ok(()),
        Err(e) => Err(AlternatorError::Mastodon(e)),
    }
}
