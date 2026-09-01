#![allow(dead_code)]

use std::f32::consts;

use crate::bi_quad_filter::BiQuadFilter;
use crate::channel::Channel;
use crate::generator_type::GeneratorType;
use crate::lfo::Lfo;
use crate::modulation_envelope::ModulationEnvelope;
use crate::modulator::Modulator;
use crate::oscillator::Oscillator;
use crate::region_ex::RegionEx;
use crate::region_pair::RegionPair;
use crate::soundfont_math::SoundFontMath;
use crate::synthesizer_settings::SynthesizerSettings;
use crate::volume_envelope::VolumeEnvelope;

#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
enum VoiceState {
    Playing = 0,
    ReleaseRequested = 1,
    Released = 2,
}

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct Voice {
    vol_env: VolumeEnvelope,
    mod_env: ModulationEnvelope,

    vib_lfo: Lfo,
    mod_lfo: Lfo,

    oscillator: Oscillator,
    filter: BiQuadFilter,

    block: Vec<f32>,

    // A sudden change in the mix gain will cause pop noise.
    // To avoid this, we save the mix gain of the previous block,
    // and smooth out the gain if the gap between the current and previous gain is too large.
    // The actual smoothing process is done in the WriteBlock method of the Synthesizer class.
    pub(crate) previous_mix_gain_left: f32,
    pub(crate) previous_mix_gain_right: f32,
    pub(crate) current_mix_gain_left: f32,
    pub(crate) current_mix_gain_right: f32,

    pub(crate) previous_reverb_send: f32,
    pub(crate) previous_chorus_send: f32,
    pub(crate) current_reverb_send: f32,
    pub(crate) current_chorus_send: f32,

    exclusive_class: i32,
    channel: i32,
    key: i32,
    velocity: i32,

    note_gain: f32,

    // Every synthesis parameter is the sum of three terms, indexed by
    // generator number: what the generators say, what the note-on-static
    // modulators contribute, and what the dynamic ones contribute right now.
    // Keeping them apart is what lets the legacy scale factors apply to the
    // generator alone - scaling the velocity curve by the 0.4 that belongs to
    // generator attenuation would flatten every SoundFont's dynamics by 60%.
    /// Preset plus instrument generators.
    gen_cb: [f32; GeneratorType::COUNT],
    /// Modulators whose sources cannot change while the voice sounds.
    static_cb: [f32; GeneratorType::COUNT],
    /// Modulators re-evaluated at the top of every block.
    dyn_cb: [f32; GeneratorType::COUNT],

    // A fixed array rather than a Vec: voices are preallocated, so a Vec would
    // heap-allocate on the audio thread at the first note-on.
    dynamic_modulators: [Modulator; Voice::MAX_DYNAMIC_MODULATORS],
    dynamic_modulator_count: usize,
    /// The destinations any dynamic modulator writes, so the per-block pass
    /// clears only those instead of 61 floats per voice.
    dynamic_destinations: [u8; Voice::MAX_DYNAMIC_MODULATORS],
    dynamic_destination_count: usize,

    dynamic_cutoff: bool,
    dynamic_resonance: bool,
    dynamic_volume: bool,

    // Some instruments require fast cutoff change, which can cause pop noise.
    // This is used to smooth out the cutoff frequency.
    smoothed_cutoff: f32,
    /// The same idea for resonance, which only became modulatable with SF2
    /// modulator support. A controller sweeping Q steps the biquad
    /// coefficients against persistent filter state, which thumps.
    smoothed_resonance_db: f32,

    reverb_send_scale: f32,
    chorus_send_scale: f32,

    voice_state: VoiceState,
    /// Time elapsed in samples
    voice_length: usize,
    min_voice_length: usize,
}

impl Voice {
    /// How many dynamic modulators one voice will track. Real fonts carry
    /// fewer than ten per region; anything past this is dropped rather than
    /// allocated for.
    const MAX_DYNAMIC_MODULATORS: usize = 32;

    /// Filter resonance may move by at most this much per block, so that a
    /// controller sweeping Q does not step the coefficients and thump.
    const MAX_RESONANCE_CHANGE_DB: f32 = 3_f32;

    pub(crate) fn new(settings: &SynthesizerSettings) -> Self {
        Self {
            vol_env: VolumeEnvelope::new(settings),
            mod_env: ModulationEnvelope::new(settings),
            vib_lfo: Lfo::new(settings),
            mod_lfo: Lfo::new(settings),
            oscillator: Oscillator::new(settings),
            filter: BiQuadFilter::new(settings),
            block: vec![0_f32; settings.block_size],
            previous_mix_gain_left: 0_f32,
            previous_mix_gain_right: 0_f32,
            current_mix_gain_left: 0_f32,
            current_mix_gain_right: 0_f32,
            previous_reverb_send: 0_f32,
            previous_chorus_send: 0_f32,
            current_reverb_send: 0_f32,
            current_chorus_send: 0_f32,
            exclusive_class: 0,
            channel: 0,
            key: 0,
            velocity: 0,
            note_gain: 0_f32,
            gen_cb: [0_f32; GeneratorType::COUNT],
            static_cb: [0_f32; GeneratorType::COUNT],
            dyn_cb: [0_f32; GeneratorType::COUNT],
            dynamic_modulators: [Modulator::inactive(); Voice::MAX_DYNAMIC_MODULATORS],
            dynamic_modulator_count: 0,
            dynamic_destinations: [0; Voice::MAX_DYNAMIC_MODULATORS],
            dynamic_destination_count: 0,
            dynamic_cutoff: false,
            dynamic_resonance: false,
            dynamic_volume: false,
            smoothed_cutoff: 0_f32,
            smoothed_resonance_db: 0_f32,
            reverb_send_scale: settings.reverb_send_scale,
            chorus_send_scale: settings.chorus_send_scale,
            voice_state: VoiceState::Playing,
            voice_length: 0,
            min_voice_length: (settings.sample_rate / 500) as usize,
        }
    }

    /// Evaluates one modulator list into the note-on accumulators.
    ///
    /// Every modulator is evaluated here, dynamic ones included, because a
    /// destination that is only read at note-on still has to see them: a
    /// modulator from CC74 to attackVolEnv has a source that changes but a
    /// destination that is latched, and splitting on the source alone would
    /// drop it entirely.
    fn accumulate_modulators(
        &mut self,
        modulators: &[Modulator],
        channel_info: &Channel,
        key: i32,
        velocity: i32,
        mod_cb: &mut [f32; GeneratorType::COUNT],
    ) {
        for modulator in modulators.iter() {
            let destination = modulator.get_destination() as usize;
            if destination >= GeneratorType::COUNT {
                continue;
            }

            let value = modulator.evaluate(channel_info, key, velocity);
            mod_cb[destination] += value;

            if modulator.is_static() {
                self.static_cb[destination] += value;
                continue;
            }

            if self.dynamic_modulator_count == Voice::MAX_DYNAMIC_MODULATORS {
                continue;
            }

            self.dynamic_modulators[self.dynamic_modulator_count] = *modulator;
            self.dynamic_modulator_count += 1;

            let seen = self.dynamic_destinations[..self.dynamic_destination_count]
                .contains(&(destination as u8));
            if !seen {
                self.dynamic_destinations[self.dynamic_destination_count] = destination as u8;
                self.dynamic_destination_count += 1;
            }
        }
    }

    /// Re-evaluates the dynamic modulators. Called once per block, never per
    /// sample: no modulator source changes faster than a block.
    fn update_dynamic_modulators(&mut self, channel_info: &Channel) {
        for i in 0..self.dynamic_destination_count {
            self.dyn_cb[self.dynamic_destinations[i] as usize] = 0_f32;
        }

        for i in 0..self.dynamic_modulator_count {
            let modulator = self.dynamic_modulators[i];
            let destination = modulator.get_destination() as usize;
            self.dyn_cb[destination] += modulator.evaluate(channel_info, self.key, self.velocity);
        }
    }

    /// The current value of a generator: what the font set, plus everything
    /// its modulators contribute.
    fn value(&self, destination: u16) -> f32 {
        let i = destination as usize;
        self.gen_cb[i] + self.static_cb[i] + self.dyn_cb[i]
    }

    /// What the modulators alone contribute to a generator.
    fn modulated(&self, destination: u16) -> f32 {
        let i = destination as usize;
        self.static_cb[i] + self.dyn_cb[i]
    }

    fn has_dynamic_destination(&self, destination: u16) -> bool {
        self.dynamic_destinations[..self.dynamic_destination_count].contains(&(destination as u8))
    }

    /// Clamps resonance to the SF2 generator range.
    ///
    /// This is not tidiness, it is the one clamp that has to be here.
    /// `BiQuadFilter::set_low_pass_filter` divides by `1 + 6 * (resonance - 1)`,
    /// which is zero at about -1.58 dB. A generator alone can never reach it,
    /// but a modulator with a negative amount can, and FluidR3_GM ships forty
    /// of them at amount -470. The NaN that follows propagates into the reverb
    /// and chorus comb filters, which are IIR with persistent state, and the
    /// output never recovers.
    fn clamp_resonance(centibels: f32) -> f32 {
        SoundFontMath::clamp(centibels, 0_f32, 960_f32)
    }

    /// Clamps attenuation to a range that cannot boost.
    ///
    /// Negative attenuation is gain, and gain applied before the mix is how
    /// clipping happens. The upper bound is 144 dB, which is inaudible either
    /// way, so this only ever changes something already silent.
    fn clamp_attenuation(centibels: f32) -> f32 {
        SoundFontMath::clamp(centibels, 0_f32, 1440_f32)
    }

    /// Clamps the modulation-LFO tremolo depth.
    ///
    /// Unclamped this is an exponent: the gain is `10^(0.05 * 0.1 * value)`,
    /// so a modulator driving it to 12000 cB asks for 1200 dB and overflows to
    /// infinity, which poisons the effect buses exactly as a NaN would. No
    /// font in the test set targets this generator at all, which is precisely
    /// why it needs a bound rather than an assumption.
    fn clamp_tremolo(centibels: f32) -> f32 {
        SoundFontMath::clamp(centibels, -960_f32, 960_f32)
    }

    /// Clamps a pitch modulation depth to ten octaves either way.
    ///
    /// Pitch feeds the oscillator's sample stepping, and this crate has a
    /// history of out-of-range values reaching sample addressing.
    fn clamp_pitch(cents: f32) -> f32 {
        SoundFontMath::clamp(cents, -12000_f32, 12000_f32)
    }

    /// Clamps the modulator contribution to pan to the SF2 generator range.
    ///
    /// A font that writes the spec's own default pan modulator amount of 1000
    /// rather than the 500 that matches this crate would otherwise double the
    /// sensitivity of CC10. TimGM6mb does exactly that.
    fn clamp_pan(centibels: f32) -> f32 {
        SoundFontMath::clamp(centibels, -500_f32, 500_f32)
    }

    /// The filter cutoff before the per-block LFO and envelope modulation.
    ///
    /// Deliberately not clamped: there is no NaN to avoid here, and fonts do
    /// use values above the spec's 13500 to mean "no filter at all", which
    /// works because the biquad leaves itself inactive above Nyquist.
    fn base_cutoff(&self) -> f32 {
        SoundFontMath::cents_to_hertz(self.value(GeneratorType::INITIAL_FILTER_CUTOFF_FREQUENCY))
    }

    pub(crate) fn start(
        &mut self,
        region: &RegionPair,
        channel_info: &Channel,
        channel: i32,
        key: i32,
        velocity: i32,
    ) {
        self.exclusive_class = region.get_exclusive_class();
        self.channel = channel;
        self.key = key;
        self.velocity = velocity;

        self.static_cb = [0_f32; GeneratorType::COUNT];
        self.dyn_cb = [0_f32; GeneratorType::COUNT];
        self.dynamic_modulator_count = 0;
        self.dynamic_destination_count = 0;

        // The instrument list already has the SF2 defaults merged into it. The
        // preset list is added on top, matching preset generators being
        // offsets. Adding is equivalent to appending because every surviving
        // modulator has the identity transform - which is exactly why the
        // others were dropped at load time.
        let mut mod_cb = [0_f32; GeneratorType::COUNT];
        let instrument_modulators = region.instrument.resolved_modulators.clone();
        let preset_modulators = region.preset.modulators.clone();
        self.accumulate_modulators(
            &instrument_modulators,
            channel_info,
            key,
            velocity,
            &mut mod_cb,
        );
        self.accumulate_modulators(&preset_modulators, channel_info, key, velocity, &mut mod_cb);

        for i in 0..GeneratorType::COUNT {
            self.gen_cb[i] = region.generator_gs(i);
        }

        // Attaching the note-on snapshot here is what carries modulators into
        // the envelopes, the LFOs and the oscillator tuning, all of which read
        // their parameters exactly once.
        let region = &region.with_modulators(mod_cb);

        if velocity > 0 {
            // According to the Polyphone's implementation, the initial attenuation should be reduced to 40%.
            // I'm not sure why, but this indeed improves the loudness variability.
            //
            // The 40% belongs to the generator alone. Velocity arrives as a
            // modulator now, and scaling that by 0.4 as well would flatten the
            // velocity curve on every SoundFont.
            let sample_attenuation =
                0.4_f32 * 0.1_f32 * self.gen_cb[GeneratorType::INITIAL_ATTENUATION as usize];
            let modulator_attenuation = 0.1_f32
                * Voice::clamp_attenuation(self.modulated(GeneratorType::INITIAL_ATTENUATION));
            let filter_attenuation = 0.5_f32
                * 0.1_f32
                * Voice::clamp_resonance(self.value(GeneratorType::INITIAL_FILTER_Q));
            let decibels = -modulator_attenuation - sample_attenuation - filter_attenuation;
            self.note_gain = SoundFontMath::decibels_to_linear(decibels);
        } else {
            // The velocity modulator would give -96 dB here rather than
            // silence, so zero velocity stays a special case.
            self.note_gain = 0_f32;
        }

        self.dynamic_cutoff = self.value(GeneratorType::MODULATION_LFO_TO_FILTER_CUTOFF_FREQUENCY)
            != 0_f32
            || self.value(GeneratorType::MODULATION_ENVELOPE_TO_FILTER_CUTOFF_FREQUENCY) != 0_f32
            || self.has_dynamic_destination(GeneratorType::INITIAL_FILTER_CUTOFF_FREQUENCY)
            || self
                .has_dynamic_destination(GeneratorType::MODULATION_LFO_TO_FILTER_CUTOFF_FREQUENCY)
            || self.has_dynamic_destination(
                GeneratorType::MODULATION_ENVELOPE_TO_FILTER_CUTOFF_FREQUENCY,
            );

        self.dynamic_resonance = self.has_dynamic_destination(GeneratorType::INITIAL_FILTER_Q);

        // The test used to be a bare `> 0.05`, which silently ignored every
        // negative modLfoToVolume. Comparing the magnitude is what the
        // generator actually means.
        self.dynamic_volume = (0.1_f32
            * Voice::clamp_tremolo(self.value(GeneratorType::MODULATION_LFO_TO_VOLUME)))
        .abs()
            > 0.05_f32
            || self.has_dynamic_destination(GeneratorType::MODULATION_LFO_TO_VOLUME);

        self.smoothed_resonance_db =
            0.1_f32 * Voice::clamp_resonance(self.value(GeneratorType::INITIAL_FILTER_Q));

        RegionEx::start_volume_envelope(&mut self.vol_env, region, key, velocity);
        RegionEx::start_modulation_envelope(&mut self.mod_env, region, key, velocity);
        RegionEx::start_vibrato(&mut self.vib_lfo, region, key, velocity);
        RegionEx::start_modulation(&mut self.mod_lfo, region, key, velocity);
        RegionEx::start_oscillator(&mut self.oscillator, region);
        self.filter.clear_buffer();

        self.smoothed_cutoff = self.base_cutoff();
        self.filter.set_low_pass_filter(
            self.smoothed_cutoff,
            SoundFontMath::decibels_to_linear(self.smoothed_resonance_db),
        );

        self.voice_state = VoiceState::Playing;
        self.voice_length = 0;
    }

    pub(crate) fn end(&mut self) {
        if self.voice_state == VoiceState::Playing {
            self.voice_state = VoiceState::ReleaseRequested;
        }
    }

    pub(crate) fn kill(&mut self) {
        self.note_gain = 0_f32;
    }

    pub(crate) fn process(&mut self, data: &[i16], channels: &[Channel]) -> bool {
        if self.note_gain < SoundFontMath::NON_AUDIBLE {
            return false;
        }

        let channel_info = &channels[self.channel as usize];

        self.release_if_necessary(channel_info);

        if !self.vol_env.process(self.block.len()) {
            return false;
        }

        self.mod_env.process(self.block.len());
        self.vib_lfo.process();
        self.mod_lfo.process();

        // Once per block, before anything reads a modulated parameter. Nothing
        // outside this method may write the mix gains, the sends or the
        // dynamic attenuation: recomputing them eagerly when a controller
        // changes would desynchronise previous_* from what write_block
        // actually rendered, and produce a step exactly the size of the
        // controller change.
        self.update_dynamic_modulators(channel_info);

        // CC1 and channel pressure reach vibrato depth as modulators now,
        // which is where the old 0.01 * get_modulation() term went.
        let vib_pitch_change = 0.01_f32
            * Voice::clamp_pitch(self.value(GeneratorType::VIBRATO_LFO_TO_PITCH))
            * self.vib_lfo.get_value();
        let mod_pitch_change = 0.01_f32
            * Voice::clamp_pitch(self.value(GeneratorType::MODULATION_LFO_TO_PITCH))
            * self.mod_lfo.get_value()
            + 0.01_f32
                * Voice::clamp_pitch(self.value(GeneratorType::MODULATION_ENVELOPE_TO_PITCH))
                * self.mod_env.get_value();
        let channel_pitch_change = channel_info.get_tune() + channel_info.get_pitch_bend();
        let pitch = self.key as f32 + vib_pitch_change + mod_pitch_change + channel_pitch_change;
        if !self.oscillator.process(data, &mut self.block[..], pitch) {
            return false;
        }

        if self.dynamic_cutoff || self.dynamic_resonance {
            let cents = self.value(GeneratorType::MODULATION_LFO_TO_FILTER_CUTOFF_FREQUENCY)
                * self.mod_lfo.get_value()
                + self.value(GeneratorType::MODULATION_ENVELOPE_TO_FILTER_CUTOFF_FREQUENCY)
                    * self.mod_env.get_value();
            let factor = SoundFontMath::cents_to_multiplying_factor(cents);
            let new_cutoff = factor * self.base_cutoff();

            // The cutoff change is limited within x0.5 and x2 to reduce pop noise.
            let lower_limit = 0.5_f32 * self.smoothed_cutoff;
            let upper_limit = 2_f32 * self.smoothed_cutoff;
            self.smoothed_cutoff = SoundFontMath::clamp(new_cutoff, lower_limit, upper_limit);

            // Resonance needs the same treatment for the same reason. It only
            // became able to move at all once modulators could drive it, and
            // stepping the coefficients against the persistent filter state
            // thumps.
            let resonance_db =
                0.1_f32 * Voice::clamp_resonance(self.value(GeneratorType::INITIAL_FILTER_Q));
            self.smoothed_resonance_db = SoundFontMath::clamp(
                resonance_db,
                self.smoothed_resonance_db - Voice::MAX_RESONANCE_CHANGE_DB,
                self.smoothed_resonance_db + Voice::MAX_RESONANCE_CHANGE_DB,
            );

            self.filter.set_low_pass_filter(
                self.smoothed_cutoff,
                SoundFontMath::decibels_to_linear(self.smoothed_resonance_db),
            );
        }
        self.filter.process(&mut self.block[..]);

        self.previous_mix_gain_left = self.current_mix_gain_left;
        self.previous_mix_gain_right = self.current_mix_gain_right;
        self.previous_reverb_send = self.current_reverb_send;
        self.previous_chorus_send = self.current_chorus_send;

        // CC7 and CC11 reach attenuation as modulators now, which is where the
        // old squared volume-times-expression term went.
        //
        // It has to land on mix_gain rather than note_gain. A fader pulled to
        // zero is about -96 dB, and note_gain below NON_AUDIBLE retires the
        // voice permanently - so folding it in would silently destroy every
        // voice on the channel until the next note-on, which for an expression
        // pedal that sweeps to zero is routine.
        let dynamic_attenuation = 0.1_f32
            * Voice::clamp_attenuation(self.dyn_cb[GeneratorType::INITIAL_ATTENUATION as usize]);
        let mut mix_gain = self.note_gain
            * SoundFontMath::decibels_to_linear(-dynamic_attenuation)
            * self.vol_env.get_value();
        if self.dynamic_volume {
            let decibels = 0.1_f32
                * Voice::clamp_tremolo(self.value(GeneratorType::MODULATION_LFO_TO_VOLUME))
                * self.mod_lfo.get_value();
            mix_gain *= SoundFontMath::decibels_to_linear(decibels);
        }

        // The instrument pan keeps its own clamp and the sum is left to
        // saturate in the branches below, exactly as before. CC10 arrives
        // through the modulator term.
        let generator_pan = SoundFontMath::clamp(
            0.1_f32 * self.gen_cb[GeneratorType::PAN as usize],
            -50_f32,
            50_f32,
        );
        let modulator_pan = 0.1_f32 * Voice::clamp_pan(self.modulated(GeneratorType::PAN));
        let angle = (consts::PI / 200_f32) * (generator_pan + modulator_pan + 50_f32);
        if angle <= 0_f32 {
            self.current_mix_gain_left = mix_gain;
            self.current_mix_gain_right = 0_f32;
        } else if angle >= SoundFontMath::HALF_PI {
            self.current_mix_gain_left = 0_f32;
            self.current_mix_gain_right = mix_gain;
        } else {
            self.current_mix_gain_left = mix_gain * angle.cos();
            self.current_mix_gain_right = mix_gain * angle.sin();
        }

        // CC91 and CC93 arrive as modulators. The send scales exist because a
        // font that ships its own send modulators overrides the defaults
        // entirely - GeneralUser GS caps reverb at 35% and chorus at 30% - and
        // a caller may want that dialled back up without patching the font.
        self.current_reverb_send = SoundFontMath::clamp(
            0.001_f32 * self.value(GeneratorType::REVERB_EFFECTS_SEND) * self.reverb_send_scale,
            0_f32,
            1_f32,
        );
        self.current_chorus_send = SoundFontMath::clamp(
            0.001_f32 * self.value(GeneratorType::CHORUS_EFFECTS_SEND) * self.chorus_send_scale,
            0_f32,
            1_f32,
        );

        if self.voice_length == 0 {
            self.previous_mix_gain_left = self.current_mix_gain_left;
            self.previous_mix_gain_right = self.current_mix_gain_right;
            self.previous_reverb_send = self.current_reverb_send;
            self.previous_chorus_send = self.current_chorus_send;
        }

        self.voice_length += self.block.len();

        true
    }

    fn release_if_necessary(&mut self, channel_info: &Channel) {
        if self.voice_length < self.min_voice_length {
            return;
        }

        if self.voice_state == VoiceState::ReleaseRequested && !channel_info.get_hold_pedal() {
            self.vol_env.release();
            self.mod_env.release();
            self.oscillator.release();

            self.voice_state = VoiceState::Released;
        }
    }

    pub(crate) fn block(&self) -> &Vec<f32> {
        &self.block
    }

    pub(crate) fn voice_length(&self) -> usize {
        self.voice_length
    }

    pub(crate) fn exclusive_class(&self) -> i32 {
        self.exclusive_class
    }

    pub(crate) fn channel(&self) -> i32 {
        self.channel
    }

    pub(crate) fn key(&self) -> i32 {
        self.key
    }

    pub(crate) fn priority(&self) -> f32 {
        if self.note_gain < SoundFontMath::NON_AUDIBLE {
            0_f32
        } else {
            self.vol_env.get_priority()
        }
    }
}
