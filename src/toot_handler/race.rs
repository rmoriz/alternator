use crate::error::{AlternatorError, MastodonError};
use crate::mastodon::MastodonStream;
use tracing::{debug, warn};

/// Check for race conditions by retrieving current toot state
pub async fn check_race_condition(
    mastodon_client: &impl MastodonStream,
    toot_id: &str,
    media_id: &str,
) -> Result<(), AlternatorError> {
    debug!(
        "Checking for race conditions on toot {} media {}",
        toot_id, media_id
    );

    match mastodon_client.get_toot(toot_id).await {
        Ok(current_toot) => {
            // Find the current state of this media attachment
            if let Some(current_media) = current_toot
                .media_attachments
                .iter()
                .find(|m| m.id == *media_id)
            {
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
            Ok(())
        }
        Err(e) => {
            warn!(
                "Could not retrieve current toot state for race condition check: {}",
                e
            );
            Err(AlternatorError::Mastodon(e))
        }
    }
}


