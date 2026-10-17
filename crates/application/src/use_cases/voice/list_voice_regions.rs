use crate::context::{Ports, VoiceRegion};

/// Regions with the default one marked optimal.
pub fn execute(ports: &Ports) -> Vec<(VoiceRegion, bool)> {
    ports
        .settings
        .voice_regions
        .iter()
        .map(|region| (region.clone(), region.id == ports.settings.default_voice_region))
        .collect()
}
