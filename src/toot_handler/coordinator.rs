use crate::error::{AlternatorError, MastodonError};
use crate::mastodon::{MastodonStream, MediaRecreation};
use crate::media::MediaProcessor;
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

    // Check if any of the original media attachments now have descriptions
    let processable_media = MediaProcessor::with_default_config()
        .filter_processable_media(&current_toot.media_attachments);

    if processable_media.len() != media_recreations.len() {
        debug!(
            "Media state changed: expected {} processable media, found {}. Race condition detected.",
            media_recreations.len(),
            processable_media.len()
        );
        return Err(AlternatorError::Mastodon(
            MastodonError::RaceConditionDetected,
        ));
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