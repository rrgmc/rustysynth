#![allow(dead_code)]

use crate::default_modulators::DEFAULT_MODULATORS;
use crate::generator::Generator;
use crate::generator_type::GeneratorType;
use crate::loop_mode::LoopMode;
use crate::modulator::Modulator;
use crate::sample_header::SampleHeader;
use crate::soundfont_math::SoundFontMath;
use crate::soundfont_warning::{SoundFontWarning, WarningCollector};
use crate::zone::Zone;

fn set_parameter(gs: &mut [i16; GeneratorType::COUNT], generator: &Generator) {
    let index = generator.generator_type as usize;

    // Unknown generators should be ignored.
    if index < gs.len() {
        gs[index] = generator.value as i16;
    }
}

/// Represents an instrument region.
/// An instrument region contains all the parameters necessary to synthesize a note.
#[derive(Debug)]
#[non_exhaustive]
pub struct InstrumentRegion {
    pub(crate) gs: [i16; GeneratorType::COUNT],
    /// The modulators this region's zones actually carry, which is what
    /// `get_modulators` exposes.
    pub(crate) modulators: Vec<Modulator>,
    /// The same list merged over the SF2 defaults, which is what a voice uses.
    /// Kept separate so that the deliberately non-spec send amounts are not
    /// presented as if the font had asked for them.
    pub(crate) resolved_modulators: Vec<Modulator>,
    pub(crate) sample_start: i32,
    pub(crate) sample_end: i32,
    pub(crate) sample_start_loop: i32,
    pub(crate) sample_end_loop: i32,
    pub(crate) sample_sample_rate: i32,
    pub(crate) sample_original_pitch: i32,
    pub(crate) sample_pitch_correction: i32,
}

impl InstrumentRegion {
    /// Builds one region, or `None` if the zone does not describe a playable
    /// one. A dropped zone is recorded rather than rejecting the whole font:
    /// one unplayable region out of thousands is not a reason to refuse a bank.
    fn new(
        instrument_id: usize,
        zone_index: usize,
        global: &Zone,
        local: &Zone,
        samples: &[SampleHeader],
        warnings: &mut WarningCollector,
    ) -> Option<Self> {
        let mut gs: [i16; GeneratorType::COUNT] = [0; GeneratorType::COUNT];
        gs[GeneratorType::INITIAL_FILTER_CUTOFF_FREQUENCY as usize] = 13500;
        gs[GeneratorType::DELAY_MODULATION_LFO as usize] = -12000;
        gs[GeneratorType::DELAY_VIBRATO_LFO as usize] = -12000;
        gs[GeneratorType::DELAY_MODULATION_ENVELOPE as usize] = -12000;
        gs[GeneratorType::ATTACK_MODULATION_ENVELOPE as usize] = -12000;
        gs[GeneratorType::HOLD_MODULATION_ENVELOPE as usize] = -12000;
        gs[GeneratorType::DECAY_MODULATION_ENVELOPE as usize] = -12000;
        gs[GeneratorType::RELEASE_MODULATION_ENVELOPE as usize] = -12000;
        gs[GeneratorType::DELAY_VOLUME_ENVELOPE as usize] = -12000;
        gs[GeneratorType::ATTACK_VOLUME_ENVELOPE as usize] = -12000;
        gs[GeneratorType::HOLD_VOLUME_ENVELOPE as usize] = -12000;
        gs[GeneratorType::DECAY_VOLUME_ENVELOPE as usize] = -12000;
        gs[GeneratorType::RELEASE_VOLUME_ENVELOPE as usize] = -12000;
        gs[GeneratorType::KEY_RANGE as usize] = 0x7F00;
        gs[GeneratorType::VELOCITY_RANGE as usize] = 0x7F00;
        gs[GeneratorType::KEY_NUMBER as usize] = -1;
        gs[GeneratorType::VELOCITY as usize] = -1;
        gs[GeneratorType::SCALE_TUNING as usize] = 100;
        gs[GeneratorType::OVERRIDING_ROOT_KEY as usize] = -1;

        for generator in global.generators.iter() {
            set_parameter(&mut gs, generator);
        }

        for generator in local.generators.iter() {
            set_parameter(&mut gs, generator);
        }

        let mut modulators: Vec<Modulator> = Vec::new();
        Modulator::merge(&mut modulators, &global.modulators);
        Modulator::merge(&mut modulators, &local.modulators);

        let mut resolved_modulators = DEFAULT_MODULATORS.to_vec();
        Modulator::merge(&mut resolved_modulators, &global.modulators);
        Modulator::merge(&mut resolved_modulators, &local.modulators);

        // SF2 2.04 section 7.7 requires a `sampleID` generator to terminate
        // every non-global instrument zone. A zone without one used to fall
        // through to the zero-initialized slot below and silently play sample
        // 0, which is both wrong and a way for a zone that names nothing to
        // resolve to addresses that fail the sanity check.
        if !local
            .generators
            .iter()
            .any(|generator| generator.generator_type == GeneratorType::SAMPLE_ID)
        {
            warnings.push(SoundFontWarning::ZoneWithoutSampleId {
                instrument_id,
                zone_index,
            });
            return None;
        }

        let sample_id = gs[GeneratorType::SAMPLE_ID as usize] as usize;
        let Some(sample) = samples.get(sample_id) else {
            warnings.push(SoundFontWarning::RegionInvalidSampleId {
                instrument_id,
                sample_id,
            });
            return None;
        };

        Some(Self {
            gs,
            modulators,
            resolved_modulators,
            sample_start: sample.start,
            sample_end: sample.end,
            sample_start_loop: sample.start_loop,
            sample_end_loop: sample.end_loop,
            sample_sample_rate: sample.sample_rate,
            sample_original_pitch: sample.original_pitch as i32,
            sample_pitch_correction: sample.pitch_correction as i32,
        })
    }

    pub(crate) fn create(
        instrument_id: usize,
        zones: &[Zone],
        samples: &[SampleHeader],
        warnings: &mut WarningCollector,
    ) -> Vec<InstrumentRegion> {
        // An instrument with an empty bag span has no regions. It used to be
        // impossible to reach here - the caller rejected the whole file first -
        // and indexing zone 0 below would panic.
        if zones.is_empty() {
            return Vec::new();
        }

        // Is the first one the global zone?
        if zones[0].generators.is_empty()
            || zones[0].generators.last().unwrap().generator_type != GeneratorType::SAMPLE_ID
        {
            // The first one is the global zone.
            let global = &zones[0];

            // The global zone is regarded as the base setting of subsequent zones.
            zones[1..]
                .iter()
                .enumerate()
                .filter_map(|(i, local)| {
                    InstrumentRegion::new(instrument_id, i + 1, global, local, samples, warnings)
                })
                .collect()
        } else {
            // No global zone.
            let empty = Zone::empty();
            zones
                .iter()
                .enumerate()
                .filter_map(|(i, local)| {
                    InstrumentRegion::new(instrument_id, i, &empty, local, samples, warnings)
                })
                .collect()
        }
    }

    /// Checks if the region covers the given key and velocity.
    /// Returns `true` if the region covers the given key and velocity.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of a note.
    /// * `velocity` - The velocity of a note.
    pub fn contains(&self, key: i32, velocity: i32) -> bool {
        let contains_key = self.get_key_range_start() <= key && key <= self.get_key_range_end();
        let contains_velocity = self.get_velocity_range_start() <= velocity
            && velocity <= self.get_velocity_range_end();
        contains_key && contains_velocity
    }

    /// Gets the modulators of the region.
    ///
    /// These are the modulators the SoundFont itself carries. The SF2 default
    /// modulators that also apply are not included, since a caller inspecting
    /// a font should see what the font says.
    pub fn get_modulators(&self) -> &[Modulator] {
        &self.modulators[..]
    }

    pub fn get_sample_start(&self) -> i32 {
        self.sample_start + self.get_start_address_offset()
    }

    pub fn get_sample_end(&self) -> i32 {
        self.sample_end + self.get_end_address_offset()
    }

    pub fn get_sample_start_loop(&self) -> i32 {
        self.sample_start_loop + self.get_start_loop_address_offset()
    }

    pub fn get_sample_end_loop(&self) -> i32 {
        self.sample_end_loop + self.get_end_loop_address_offset()
    }

    pub fn get_start_address_offset(&self) -> i32 {
        32768 * self.gs[GeneratorType::START_ADDRESS_COARSE_OFFSET as usize] as i32
            + self.gs[GeneratorType::START_ADDRESS_OFFSET as usize] as i32
    }

    pub fn get_end_address_offset(&self) -> i32 {
        32768 * self.gs[GeneratorType::END_ADDRESS_COARSE_OFFSET as usize] as i32
            + self.gs[GeneratorType::END_ADDRESS_OFFSET as usize] as i32
    }

    pub fn get_start_loop_address_offset(&self) -> i32 {
        32768 * self.gs[GeneratorType::START_LOOP_ADDRESS_COARSE_OFFSET as usize] as i32
            + self.gs[GeneratorType::START_LOOP_ADDRESS_OFFSET as usize] as i32
    }

    pub fn get_end_loop_address_offset(&self) -> i32 {
        32768 * self.gs[GeneratorType::END_LOOP_ADDRESS_COARSE_OFFSET as usize] as i32
            + self.gs[GeneratorType::END_LOOP_ADDRESS_OFFSET as usize] as i32
    }

    pub fn get_modulation_lfo_to_pitch(&self) -> i32 {
        self.gs[GeneratorType::MODULATION_LFO_TO_PITCH as usize] as i32
    }

    pub fn get_vibrato_lfo_to_pitch(&self) -> i32 {
        self.gs[GeneratorType::VIBRATO_LFO_TO_PITCH as usize] as i32
    }

    pub fn get_modulation_envelope_to_pitch(&self) -> i32 {
        self.gs[GeneratorType::MODULATION_ENVELOPE_TO_PITCH as usize] as i32
    }

    pub fn get_initial_filter_cutoff_frequency(&self) -> f32 {
        SoundFontMath::cents_to_hertz(
            self.gs[GeneratorType::INITIAL_FILTER_CUTOFF_FREQUENCY as usize] as f32,
        )
    }

    pub fn get_initial_filter_q(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::INITIAL_FILTER_Q as usize] as f32
    }

    pub fn get_modulation_lfo_to_filter_cutoff_frequency(&self) -> i32 {
        self.gs[GeneratorType::MODULATION_LFO_TO_FILTER_CUTOFF_FREQUENCY as usize] as i32
    }

    pub fn get_modulation_envelope_to_filter_cutoff_frequency(&self) -> i32 {
        self.gs[GeneratorType::MODULATION_ENVELOPE_TO_FILTER_CUTOFF_FREQUENCY as usize] as i32
    }

    pub fn get_modulation_lfo_to_volume(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::MODULATION_LFO_TO_VOLUME as usize] as f32
    }

    pub fn get_chorus_effects_send(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::CHORUS_EFFECTS_SEND as usize] as f32
    }

    pub fn get_reverb_effects_send(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::REVERB_EFFECTS_SEND as usize] as f32
    }

    pub fn get_pan(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::PAN as usize] as f32
    }

    pub fn get_delay_modulation_lfo(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DELAY_MODULATION_LFO as usize] as f32,
        )
    }

    pub fn get_frequency_modulation_lfo(&self) -> f32 {
        SoundFontMath::cents_to_hertz(
            self.gs[GeneratorType::FREQUENCY_MODULATION_LFO as usize] as f32,
        )
    }

    pub fn get_delay_vibrato_lfo(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DELAY_VIBRATO_LFO as usize] as f32,
        )
    }

    pub fn get_frequency_vibrato_lfo(&self) -> f32 {
        SoundFontMath::cents_to_hertz(self.gs[GeneratorType::FREQUENCY_VIBRATO_LFO as usize] as f32)
    }

    pub fn get_delay_modulation_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DELAY_MODULATION_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_attack_modulation_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::ATTACK_MODULATION_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_hold_modulation_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::HOLD_MODULATION_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_decay_modulation_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DECAY_MODULATION_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_sustain_modulation_envelope(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::SUSTAIN_MODULATION_ENVELOPE as usize] as f32
    }

    pub fn get_release_modulation_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::RELEASE_MODULATION_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_key_number_to_modulation_envelope_hold(&self) -> i32 {
        self.gs[GeneratorType::KEY_NUMBER_TO_MODULATION_ENVELOPE_HOLD as usize] as i32
    }

    pub fn get_key_number_to_modulation_envelope_decay(&self) -> i32 {
        self.gs[GeneratorType::KEY_NUMBER_TO_MODULATION_ENVELOPE_DECAY as usize] as i32
    }

    pub fn get_delay_volume_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DELAY_VOLUME_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_attack_volume_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::ATTACK_VOLUME_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_hold_volume_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::HOLD_VOLUME_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_decay_volume_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::DECAY_VOLUME_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_sustain_volume_envelope(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::SUSTAIN_VOLUME_ENVELOPE as usize] as f32
    }

    pub fn get_release_volume_envelope(&self) -> f32 {
        SoundFontMath::timecents_to_seconds(
            self.gs[GeneratorType::RELEASE_VOLUME_ENVELOPE as usize] as f32,
        )
    }

    pub fn get_key_number_to_volume_envelope_hold(&self) -> i32 {
        self.gs[GeneratorType::KEY_NUMBER_TO_VOLUME_ENVELOPE_HOLD as usize] as i32
    }

    pub fn get_key_number_to_volume_envelope_decay(&self) -> i32 {
        self.gs[GeneratorType::KEY_NUMBER_TO_VOLUME_ENVELOPE_DECAY as usize] as i32
    }

    pub fn get_key_range_start(&self) -> i32 {
        self.gs[GeneratorType::KEY_RANGE as usize] as i32 & 0xFF
    }

    pub fn get_key_range_end(&self) -> i32 {
        (self.gs[GeneratorType::KEY_RANGE as usize] as i32 >> 8) & 0xFF
    }

    pub fn get_velocity_range_start(&self) -> i32 {
        self.gs[GeneratorType::VELOCITY_RANGE as usize] as i32 & 0xFF
    }

    pub fn get_velocity_range_end(&self) -> i32 {
        (self.gs[GeneratorType::VELOCITY_RANGE as usize] as i32 >> 8) & 0xFF
    }

    pub fn get_initial_attenuation(&self) -> f32 {
        0.1_f32 * self.gs[GeneratorType::INITIAL_ATTENUATION as usize] as f32
    }

    pub fn get_coarse_tune(&self) -> i32 {
        self.gs[GeneratorType::COARSE_TUNE as usize] as i32
    }

    pub fn get_fine_tune(&self) -> i32 {
        self.gs[GeneratorType::FINE_TUNE as usize] as i32 + self.sample_pitch_correction
    }

    pub fn get_sample_modes(&self) -> LoopMode {
        LoopMode::from_i16(self.gs[GeneratorType::SAMPLE_MODES as usize])
    }

    pub fn get_scale_tuning(&self) -> i32 {
        self.gs[GeneratorType::SCALE_TUNING as usize] as i32
    }

    pub fn get_exclusive_class(&self) -> i32 {
        self.gs[GeneratorType::EXCLUSIVE_CLASS as usize] as i32
    }

    pub fn get_root_key(&self) -> i32 {
        if self.gs[GeneratorType::OVERRIDING_ROOT_KEY as usize] != -1 {
            self.gs[GeneratorType::OVERRIDING_ROOT_KEY as usize] as i32
        } else {
            self.sample_original_pitch
        }
    }

    pub fn get_sample_id(&self) -> usize {
        self.gs[GeneratorType::SAMPLE_ID as usize] as usize
    }
}
