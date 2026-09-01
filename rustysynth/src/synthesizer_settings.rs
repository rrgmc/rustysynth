#![allow(dead_code)]

use crate::error::SynthesizerError;

/// Specifies a set of parameters for synthesis.
#[derive(Debug)]
#[non_exhaustive]
pub struct SynthesizerSettings {
    /// The sample rate for synthesis.
    pub sample_rate: i32,
    /// The block size for rendering waveform.
    pub block_size: usize,
    /// The number of maximum polyphony.
    pub maximum_polyphony: usize,
    /// The value indicating whether reverb and chorus are enabled.
    pub enable_reverb_and_chorus: bool,
    /// Scales how much every voice sends to the reverb.
    ///
    /// A SoundFont that ships its own CC91 modulators overrides the default
    /// one entirely, and some cap the send well below full scale - GeneralUser
    /// GS stops at 35%. This exists so that a drier mix than intended can be
    /// brought back up without editing the font. 1.0 honors the font.
    pub reverb_send_scale: f32,
    /// Scales how much every voice sends to the chorus. See
    /// `reverb_send_scale`.
    pub chorus_send_scale: f32,
}

impl SynthesizerSettings {
    const DEFAULT_BLOCK_SIZE: usize = 64;
    const DEFAULT_MAXIMUM_POLYPHONY: usize = 64;
    const DEFAULT_ENABLE_REVERB_AND_CHORUS: bool = true;
    const DEFAULT_SEND_SCALE: f32 = 1_f32;
    const MAXIMUM_SEND_SCALE: f32 = 10_f32;

    /// Initializes a new instance of synthesizer settings.
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - The sample rate for synthesis.
    pub fn new(sample_rate: i32) -> Self {
        Self {
            sample_rate,
            block_size: SynthesizerSettings::DEFAULT_BLOCK_SIZE,
            maximum_polyphony: SynthesizerSettings::DEFAULT_MAXIMUM_POLYPHONY,
            enable_reverb_and_chorus: SynthesizerSettings::DEFAULT_ENABLE_REVERB_AND_CHORUS,
            reverb_send_scale: SynthesizerSettings::DEFAULT_SEND_SCALE,
            chorus_send_scale: SynthesizerSettings::DEFAULT_SEND_SCALE,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), SynthesizerError> {
        SynthesizerSettings::check_sample_rate(self.sample_rate)?;
        SynthesizerSettings::check_block_size(self.block_size)?;
        SynthesizerSettings::check_maximum_polyphony(self.maximum_polyphony)?;
        SynthesizerSettings::check_send_scale(self.reverb_send_scale)?;
        SynthesizerSettings::check_send_scale(self.chorus_send_scale)?;

        Ok(())
    }

    fn check_send_scale(value: f32) -> Result<(), SynthesizerError> {
        // Rejecting NaN matters: it would reach the effect buses, which are
        // IIR with persistent state, and never wash out.
        if !(0_f32..=SynthesizerSettings::MAXIMUM_SEND_SCALE).contains(&value) {
            return Err(SynthesizerError::SendScaleOutOfRange(value));
        }

        Ok(())
    }

    fn check_sample_rate(value: i32) -> Result<(), SynthesizerError> {
        if !(16_000..=192_000).contains(&value) {
            return Err(SynthesizerError::SampleRateOutOfRange(value));
        }

        Ok(())
    }

    fn check_block_size(value: usize) -> Result<(), SynthesizerError> {
        if !(8..=1024).contains(&value) {
            return Err(SynthesizerError::BlockSizeOutOfRange(value));
        }

        Ok(())
    }

    fn check_maximum_polyphony(value: usize) -> Result<(), SynthesizerError> {
        if !(8..=256).contains(&value) {
            return Err(SynthesizerError::MaximumPolyphonyOutOfRange(value));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_scales_default_to_honoring_the_font() {
        let settings = SynthesizerSettings::new(44100);
        assert_eq!(settings.reverb_send_scale, 1_f32);
        assert_eq!(settings.chorus_send_scale, 1_f32);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn send_scales_are_range_checked() {
        for value in [-1_f32, 11_f32, f32::NAN, f32::INFINITY] {
            let mut settings = SynthesizerSettings::new(44100);
            settings.reverb_send_scale = value;
            assert!(
                matches!(
                    settings.validate(),
                    Err(SynthesizerError::SendScaleOutOfRange(_))
                ),
                "reverb_send_scale {value} should have been rejected"
            );

            let mut settings = SynthesizerSettings::new(44100);
            settings.chorus_send_scale = value;
            assert!(matches!(
                settings.validate(),
                Err(SynthesizerError::SendScaleOutOfRange(_))
            ));
        }

        let mut settings = SynthesizerSettings::new(44100);
        settings.reverb_send_scale = 0_f32;
        settings.chorus_send_scale = 10_f32;
        assert!(settings.validate().is_ok());
    }
}
