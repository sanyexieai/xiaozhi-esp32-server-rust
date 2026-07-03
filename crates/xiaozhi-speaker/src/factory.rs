use std::sync::Arc;

use xiaozhi_config::VoiceIdentifyConfig;
use xiaozhi_core::Result;

use crate::asr_server::AsrServerSpeakerProvider;
use crate::traits::SpeakerProvider;

pub fn create_speaker(cfg: &VoiceIdentifyConfig) -> Result<Option<Arc<dyn SpeakerProvider>>> {
    if !cfg.enable || cfg.base_url.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(AsrServerSpeakerProvider::new(
        cfg.base_url.clone(),
        cfg.threshold,
    )?)))
}
