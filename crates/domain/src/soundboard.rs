use crate::Snowflake;

/// The maximum number of stored sounds in one guild.
pub const MAX_SOUNDS_PER_GUILD: usize = 8;
/// Five seconds of mono 48 kHz signed 16-bit PCM, plus the canonical WAV header.
pub const MAX_SOUND_BYTES: usize = 44 + 5 * 48_000 * 2;

#[derive(Debug, Clone, PartialEq)]
pub struct SoundboardSound {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub creator_id: Option<Snowflake>,
    pub name: String,
    pub volume: f32,
    pub audio: Vec<u8>,
}

/// Accept only canonical RIFF/WAVE PCM data with a bounded decoded duration.
/// Parsing fixed fields and checking the entire file length prevents a forged
/// duration or oversized chunk from bypassing the limit.
pub fn valid_audio(bytes: &[u8]) -> bool {
    if bytes.len() < 46 || bytes.len() > MAX_SOUND_BYTES {
        return false;
    }
    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }
    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }
    &bytes[0..4] == b"RIFF"
        && u32_at(bytes, 4) as usize == bytes.len() - 8
        && &bytes[8..12] == b"WAVE"
        && &bytes[12..16] == b"fmt "
        && u32_at(bytes, 16) == 16
        && u16_at(bytes, 20) == 1
        && u16_at(bytes, 22) == 1
        && u32_at(bytes, 24) == 48_000
        && u32_at(bytes, 28) == 96_000
        && u16_at(bytes, 32) == 2
        && u16_at(bytes, 34) == 16
        && &bytes[36..40] == b"data"
        && u32_at(bytes, 40) as usize == bytes.len() - 44
        && (bytes.len() - 44).is_multiple_of(2)
}

pub fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn valid_volume(volume: f32) -> bool {
    volume.is_finite() && (0.0..=1.0).contains(&volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: usize) -> Vec<u8> {
        let mut data = Vec::new();
        let size = (samples * 2) as u32;
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + size).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&48_000u32.to_le_bytes());
        data.extend_from_slice(&96_000u32.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        data.extend_from_slice(b"data");
        data.extend_from_slice(&size.to_le_bytes());
        data.resize(44 + size as usize, 0);
        data
    }

    #[test]
    fn wav_is_bounded_and_lengths_are_checked() {
        assert!(valid_audio(&wav(1)));
        assert!(valid_audio(&wav(5 * 48_000)));
        assert!(!valid_audio(&wav(5 * 48_000 + 1)));
        let mut forged = wav(2);
        forged[40] = 2;
        assert!(!valid_audio(&forged));
        assert!(!valid_audio(&wav(0)));
    }

    #[test]
    fn name_and_volume_reject_invalid_values() {
        assert!(valid_name("air_horn"));
        assert!(!valid_name("a"));
        assert!(!valid_name("bad name"));
        assert!(valid_volume(1.0));
        assert!(!valid_volume(f32::NAN));
        assert!(!valid_volume(1.01));
    }
}
