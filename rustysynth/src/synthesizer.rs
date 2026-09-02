#![allow(dead_code)]

use std::cmp;
use std::collections::HashMap;
use std::sync::Arc;

use crate::array_math::ArrayMath;
use crate::channel::Channel;
use crate::chorus::Chorus;
use crate::error::SynthesizerError;
use crate::region_pair::RegionPair;
use crate::reverb::Reverb;
use crate::soundfont::SoundFont;
use crate::soundfont_math::SoundFontMath;
use crate::synthesizer_settings::SynthesizerSettings;
use crate::voice_collection::VoiceCollection;

/// An instance of the SoundFont synthesizer.
#[derive(Debug)]
#[non_exhaustive]
pub struct Synthesizer {
    pub(crate) sound_font: Arc<SoundFont>,
    pub(crate) sample_rate: i32,
    pub(crate) block_size: usize,
    pub(crate) maximum_polyphony: usize,

    preset_lookup: HashMap<i32, usize>,
    default_preset: usize,

    channels: Vec<Channel>,

    voices: VoiceCollection,

    block_left: Vec<f32>,
    block_right: Vec<f32>,

    inverse_block_size: f32,

    block_read: usize,

    master_volume: f32,

    effects: Option<Effects>,
}

impl Synthesizer {
    /// The number of channels.
    pub const CHANNEL_COUNT: usize = 16;
    /// The percussion channel.
    pub const PERCUSSION_CHANNEL: usize = 9;

    /// Initializes a new synthesizer using a specified SoundFont and settings.
    ///
    /// # Arguments
    ///
    /// * `sound_font` - The SoundFont instance.
    /// * `settings` - The settings for synthesis.
    pub fn new(
        sound_font: &Arc<SoundFont>,
        settings: &SynthesizerSettings,
    ) -> Result<Self, SynthesizerError> {
        settings.validate()?;

        let mut preset_lookup: HashMap<i32, usize> = HashMap::new();

        let mut min_preset_id = i32::MAX;
        let mut default_preset: usize = 0;
        for i in 0..sound_font.presets.len() {
            let preset = &sound_font.presets[i];

            // The preset ID is Int32, where the upper 16 bits represent the bank number
            // and the lower 16 bits represent the patch number.
            // This ID is used to search for presets by the combination of bank number
            // and patch number.
            let preset_id = (preset.bank_number << 16) | preset.patch_number;
            preset_lookup.insert(preset_id, i);

            // The preset with the minimum ID number will be default.
            // If the SoundFont is GM compatible, the piano will be chosen.
            if preset_id < min_preset_id {
                default_preset = i;
                min_preset_id = preset_id;
            }
        }

        let mut channels: Vec<Channel> = Vec::new();
        for i in 0..Synthesizer::CHANNEL_COUNT {
            channels.push(Channel::new(i == Synthesizer::PERCUSSION_CHANNEL));
        }

        let voices = VoiceCollection::new(settings);

        let block_left: Vec<f32> = vec![0_f32; settings.block_size];
        let block_right: Vec<f32> = vec![0_f32; settings.block_size];

        let inverse_block_size = 1_f32 / settings.block_size as f32;

        let block_read = settings.block_size;

        let master_volume = 0.5_f32;

        let effects = if settings.enable_reverb_and_chorus {
            Some(Effects::new(settings))
        } else {
            None
        };

        Ok(Self {
            sound_font: Arc::clone(sound_font),
            sample_rate: settings.sample_rate,
            block_size: settings.block_size,
            maximum_polyphony: settings.maximum_polyphony,
            preset_lookup,
            default_preset,
            channels,
            voices,
            block_left,
            block_right,
            inverse_block_size,
            block_read,
            master_volume,
            effects,
        })
    }

    /// Processes a MIDI message.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel to which the message will be sent.
    /// * `command` - The type of the message.
    /// * `data1` - The first data part of the message.
    /// * `data2` - The second data part of the message.
    pub fn process_midi_message(&mut self, channel: i32, command: i32, data1: i32, data2: i32) {
        if !(0 <= channel && channel < self.channels.len() as i32) {
            return;
        }

        // Record every controller before anything else looks at the channel.
        // A SoundFont modulator may name a controller this synthesizer has no
        // dedicated field for, and the ones it does have are still handled
        // below. This has to be its own statement: three of the controller
        // cases dispatch to &mut self methods, so it cannot share a borrow
        // with the binding underneath.
        if command == 0xB0 {
            self.channels[channel as usize].set_cc(data1, data2);
        }

        let channel_info = &mut self.channels[channel as usize];

        match command {
            0x80 => self.note_off(channel, data1),       // Note Off
            0x90 => self.note_on(channel, data1, data2), // Note On
            0xB0 => match data1 // Controller
            {
                0x00 => channel_info.set_bank(data2), // Bank Selection
                0x01 => channel_info.set_modulation_coarse(data2), // Modulation Coarse
                0x21 => channel_info.set_modulation_fine(data2), // Modulation Fine
                0x06 => channel_info.data_entry_coarse(data2), // Data Entry Coarse
                0x26 => channel_info.data_entry_fine(data2), // Data Entry Fine
                0x07 => channel_info.set_volume_coarse(data2), // Channel Volume Coarse
                0x27 => channel_info.set_volume_fine(data2), // Channel Volume Fine
                0x0A => channel_info.set_pan_coarse(data2), // Pan Coarse
                0x2A => channel_info.set_pan_fine(data2), // Pan Fine
                0x0B => channel_info.set_expression_coarse(data2), // Expression Coarse
                0x2B => channel_info.set_expression_fine(data2), // Expression Fine
                0x40 => channel_info.set_hold_pedal(data2), // Hold Pedal
                0x5B => channel_info.set_reverb_send(data2), // Reverb Send
                0x5D => channel_info.set_chorus_send(data2), // Chorus Send
                0x63 => channel_info.set_nrpn_coarse(data2), // NRPN Coarse
                0x62 => channel_info.set_nrpn_fine(data2), // NRPN Fine
                0x65 => channel_info.set_rpn_coarse(data2), // RPN Coarse
                0x64 => channel_info.set_rpn_fine(data2), // RPN Fine
                0x78 => self.note_off_all_channel(channel, true), // All Sound Off
                0x79 => self.reset_all_controllers_channel(channel), // Reset All Controllers
                0x7B => self.note_off_all_channel(channel, false), // All Note Off
                // Omni Off and Omni On say nothing to a synthesizer whose
                // channels are already independent, and in particular they do
                // not select mono or poly - omni and mono/poly are orthogonal
                // bits of the MIDI mode, so forcing poly here would break the
                // conventional CC124 + CC126 pair for mode 4. What does apply
                // is that the spec makes all four mode messages act as All
                // Notes Off.
                0x7C | 0x7D => self.note_off_all_channel(channel, false), // Omni Off / Omni On
                0x7E => self.set_channel_mono_mode(channel, true), // Mono Mode On
                0x7F => self.set_channel_mono_mode(channel, false), // Poly Mode On
                _ => (),
            },
            0xA0 => channel_info.set_poly_pressure(data1, data2), // Polyphonic Key Pressure
            0xC0 => channel_info.set_patch(data1),                // Program Change
            0xD0 => channel_info.set_channel_pressure(data1),     // Channel Pressure
            0xE0 => channel_info.set_pitch_bend(data1, data2),    // Pitch Bend
            _ => (),
        }
    }

    /// Stops a note.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel of the note.
    /// * `key` - The key of the note.
    pub fn note_off(&mut self, channel: i32, key: i32) {
        if !(0 <= channel && channel < self.channels.len() as i32) {
            return;
        }

        for voice in self.voices.get_active_voices().iter_mut() {
            if voice.channel() == channel && voice.key() == key {
                voice.end();
            }
        }

        // Mono mode, last-note priority: releasing the key the channel is
        // sounding hands it back to the newest key still held. Ebony and Ivory
        // is why this is not optional - it nests a 60 ms grace note inside an
        // 800 ms sustained one, so without the fallback the sustained note is
        // stopped by the grace note and never returns, and the line loses most
        // of a bar. Releasing a key that is *not* the sounding one only
        // forgets it: the loop above matched no voice, because a mono channel
        // only ever has voices for its top key.
        //
        // This re-attacks rather than gliding to the older key. True legato
        // would mean changing an existing voice's pitch, and `Voice::start` is
        // the only place a voice's key, envelopes, LFOs and oscillator are set.
        if self.channels[channel as usize].get_mono_mode() {
            let was_sounding =
                self.channels[channel as usize].mono_top().map(|top| top.0) == Some(key);
            self.channels[channel as usize].mono_remove(key);

            if was_sounding {
                if let Some((fallback_key, velocity)) = self.channels[channel as usize].mono_top() {
                    self.note_on(channel, fallback_key, velocity);
                }
            }
        }
    }

    /// Starts a note.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel of the note.
    /// * `key` - The key of the note.
    /// * `velocity` - The velocity of the note.
    pub fn note_on(&mut self, channel: i32, key: i32, velocity: i32) {
        if velocity == 0 {
            self.note_off(channel, key);
            return;
        }

        if !(0 <= channel && channel < self.channels.len() as i32) {
            return;
        }

        let channel_info = &self.channels[channel as usize];

        // MIDI mono mode (CC126): the channel plays one note at a time, so
        // whatever it is sounding ends before the new note starts. Karaoke and
        // sequencer files write slurred monophonic leads with each note-on
        // landing before the previous note-off, which rendered polyphonically
        // is a dyad rather than a legato line.
        //
        // Three things about the placement are load-bearing. It is below the
        // `velocity == 0` early return above, because files use running-status
        // note-on-with-velocity-0 as note-off constantly and above it every
        // note-off would silence the channel. It is once per note-on rather
        // than inside the region loop below, because one note-on starts a voice
        // per matching instrument region - so "the previous note" is a set of
        // voices, and doing this per region would end voices this same note-on
        // had just started. And it releases rather than kills: `kill` drops the
        // voice with no release at all, which is a click on every note of a
        // legato line.
        if channel_info.get_mono_mode() {
            for voice in self.voices.get_active_voices().iter_mut() {
                if voice.channel() == channel {
                    voice.end();
                }
            }

            self.channels[channel as usize].mono_push(key, velocity);
        }

        let channel_info = &self.channels[channel as usize];

        let preset_id = (channel_info.get_bank_number() << 16) | channel_info.get_patch_number();

        let mut preset = self.default_preset;
        match self.preset_lookup.get(&preset_id) {
            Some(value) => preset = *value,
            None => {
                // Try fallback to the GM sound set.
                // Normally, the given patch number + the bank number 0 will work.
                // For drums (bank number >= 128), it seems to be better to select the standard set (128:0).
                let gm_preset_id = if channel_info.get_bank_number() < 128 {
                    channel_info.get_patch_number()
                } else {
                    128 << 16
                };

                // If no corresponding preset was found. Use the default one...
                if let Some(value) = self.preset_lookup.get(&gm_preset_id) {
                    preset = *value
                }
            }
        }

        let preset = &self.sound_font.presets[preset];
        for preset_region in preset.regions.iter() {
            if preset_region.contains(key, velocity) {
                let instrument = &self.sound_font.instruments[preset_region.instrument];
                for instrument_region in instrument.regions.iter() {
                    if instrument_region.contains(key, velocity) {
                        let region_pair = RegionPair::new(preset_region, instrument_region);

                        if let Some(value) = self.voices.request_new(instrument_region, channel) {
                            value.start(&region_pair, channel_info, channel, key, velocity)
                        }
                    }
                }
            }
        }
    }

    /// Stops all the notes in the specified channel.
    ///
    /// # Arguments
    ///
    /// * `immediate` - If `true`, notes will stop immediately without the release sound.
    pub fn note_off_all(&mut self, immediate: bool) {
        // Nothing is held anywhere afterwards, so no mono channel has anything
        // to fall back to.
        for state in &mut self.channels {
            state.mono_clear();
        }

        if immediate {
            self.voices.clear();
        } else {
            for voice in self.voices.get_active_voices().iter_mut() {
                voice.end();
            }
        }
    }

    /// Stops all the notes in the specified channel.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel in which the notes will be stopped.
    /// * `immediate` - If `true`, notes will stop immediately without the release sound.
    pub fn note_off_all_channel(&mut self, channel: i32, immediate: bool) {
        // Nothing is held on the channel afterwards, so a mono channel has
        // nothing to fall back to either.
        if let Some(state) = self.channels.get_mut(channel as usize) {
            state.mono_clear();
        }

        if immediate {
            for voice in self.voices.get_active_voices().iter_mut() {
                if voice.channel() == channel {
                    voice.kill();
                }
            }
        } else {
            for voice in self.voices.get_active_voices().iter_mut() {
                if voice.channel() == channel {
                    voice.end();
                }
            }
        }
    }

    /// Resets all the controllers.
    pub fn reset_all_controllers(&mut self) {
        for channel in &mut self.channels {
            channel.reset_all_controllers();
        }
    }

    /// Resets all the controllers of the specified channel.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel to be reset.
    pub fn reset_all_controllers_channel(&mut self, channel: i32) {
        if !(0 <= channel && channel < self.channels.len() as i32) {
            return;
        }

        self.channels[channel as usize].reset_all_controllers();
    }

    /// Mono Mode On (CC126) or Poly Mode On (CC127).
    ///
    /// Both also act as All Notes Off, which the MIDI spec requires of every
    /// mode message and which a file switching mode mid-phrase relies on.
    /// Release rather than kill: this is a note ending, not a note being cut.
    ///
    /// `channel` is bounds-checked by the only caller before the match it is
    /// dispatched from, which is what makes the index below safe.
    fn set_channel_mono_mode(&mut self, channel: i32, mono: bool) {
        self.channels[channel as usize].set_mono_mode(mono);
        self.note_off_all_channel(channel, false);
    }

    /// Resets the synthesizer.
    pub fn reset(&mut self) {
        self.voices.clear();

        for channel in &mut self.channels {
            channel.reset();
        }

        if let Some(effects) = self.effects.as_mut() {
            effects.reverb.mute();
            effects.chorus.mute();
        }

        self.block_read = self.block_size;
    }

    /// Renders the waveform.
    ///
    /// # Arguments
    ///
    /// * `left` - The buffer of the left channel to store the rendered waveform.
    /// * `right` - The buffer of the right channel to store the rendered waveform.
    ///
    /// # Remarks
    ///
    /// The output buffers for the left and right must be the same length.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        if left.len() != right.len() {
            panic!("The output buffers for the left and right must be the same length.");
        }

        let left_length = left.len();

        let mut wrote = 0;
        while wrote < left_length {
            if self.block_read == self.block_size {
                self.render_block();
                self.block_read = 0;
            }

            let src_rem = self.block_size - self.block_read;
            let dst_rem = left_length - wrote;
            let rem = cmp::min(src_rem, dst_rem);

            for t in 0..rem {
                left[wrote + t] = self.block_left[self.block_read + t];
                right[wrote + t] = self.block_right[self.block_read + t];
            }

            self.block_read += rem;
            wrote += rem;
        }
    }

    fn render_block(&mut self) {
        self.voices
            .process(&self.sound_font.wave_data, &self.channels);

        self.block_left.fill(0_f32);
        self.block_right.fill(0_f32);
        for voice in self.voices.get_active_voices().iter_mut() {
            let previous_gain_left = self.master_volume * voice.previous_mix_gain_left;
            let current_gain_left = self.master_volume * voice.current_mix_gain_left;
            Synthesizer::write_block(
                previous_gain_left,
                current_gain_left,
                voice.block(),
                &mut self.block_left[..],
                self.inverse_block_size,
            );
            let previous_gain_right = self.master_volume * voice.previous_mix_gain_right;
            let current_gain_right = self.master_volume * voice.current_mix_gain_right;
            Synthesizer::write_block(
                previous_gain_right,
                current_gain_right,
                voice.block(),
                &mut self.block_right[..],
                self.inverse_block_size,
            );
        }

        if let Some(effects) = self.effects.as_mut() {
            let chorus = &mut effects.chorus;
            let chorus_input_left = &mut effects.chorus_input_left[..];
            let chorus_input_right = &mut effects.chorus_input_right[..];
            let chorus_output_left = &mut effects.chorus_output_left[..];
            let chorus_output_right = &mut effects.chorus_output_right[..];
            chorus_input_left.fill(0_f32);
            chorus_input_right.fill(0_f32);
            for voice in self.voices.get_active_voices().iter_mut() {
                let previous_gain_left = voice.previous_chorus_send * voice.previous_mix_gain_left;
                let current_gain_left = voice.current_chorus_send * voice.current_mix_gain_left;
                Synthesizer::write_block(
                    previous_gain_left,
                    current_gain_left,
                    voice.block(),
                    chorus_input_left,
                    self.inverse_block_size,
                );
                let previous_gain_right =
                    voice.previous_chorus_send * voice.previous_mix_gain_right;
                let current_gain_right = voice.current_chorus_send * voice.current_mix_gain_right;
                Synthesizer::write_block(
                    previous_gain_right,
                    current_gain_right,
                    voice.block(),
                    chorus_input_right,
                    self.inverse_block_size,
                );
            }
            chorus.process(
                chorus_input_left,
                chorus_input_right,
                chorus_output_left,
                chorus_output_right,
            );
            ArrayMath::multiply_add(
                self.master_volume,
                chorus_output_left,
                &mut self.block_left[..],
            );
            ArrayMath::multiply_add(
                self.master_volume,
                chorus_output_right,
                &mut self.block_right[..],
            );

            let reverb = &mut effects.reverb;
            let reverb_input = &mut effects.reverb_input[..];
            let reverb_output_left = &mut effects.reverb_output_left[..];
            let reverb_output_right = &mut effects.reverb_output_right[..];
            reverb_input.fill(0_f32);
            for voice in self.voices.get_active_voices().iter_mut() {
                let previous_gain = reverb.get_input_gain()
                    * voice.previous_reverb_send
                    * (voice.previous_mix_gain_left + voice.previous_mix_gain_right);
                let current_gain = reverb.get_input_gain()
                    * voice.current_reverb_send
                    * (voice.current_mix_gain_left + voice.current_mix_gain_right);
                Synthesizer::write_block(
                    previous_gain,
                    current_gain,
                    voice.block(),
                    &mut reverb_input[..],
                    self.inverse_block_size,
                );
            }

            reverb.process(reverb_input, reverb_output_left, reverb_output_right);
            ArrayMath::multiply_add(
                self.master_volume,
                reverb_output_left,
                &mut self.block_left[..],
            );
            ArrayMath::multiply_add(
                self.master_volume,
                reverb_output_right,
                &mut self.block_right[..],
            );
        }
    }

    fn write_block(
        previous_gain: f32,
        current_gain: f32,
        source: &[f32],
        destination: &mut [f32],
        inverse_block_size: f32,
    ) {
        if SoundFontMath::max(previous_gain, current_gain) < SoundFontMath::NON_AUDIBLE {
            return;
        }

        if (current_gain - previous_gain).abs() < 1.0E-3_f32 {
            ArrayMath::multiply_add(current_gain, source, destination);
        } else {
            let step = inverse_block_size * (current_gain - previous_gain);
            ArrayMath::multiply_add_slope(previous_gain, step, source, destination);
        }
    }

    /// Gets the SoundFont used as the audio source.
    pub fn get_sound_font(&self) -> &SoundFont {
        &self.sound_font
    }

    /// Gets the sample rate for synthesis.
    pub fn get_sample_rate(&self) -> i32 {
        self.sample_rate
    }

    /// Gets the block size for rendering waveform.
    pub fn get_block_size(&self) -> usize {
        self.block_size
    }

    /// Gets the number of maximum polyphony.
    pub fn get_maximum_polyphony(&self) -> usize {
        self.maximum_polyphony
    }

    /// Gets the value indicating whether reverb and chorus are enabled.
    pub fn get_enable_reverb_and_chorus(&self) -> bool {
        self.effects.is_some()
    }

    /// Gets the number of voices currently sounding.
    ///
    /// A single note-on starts one voice per matching region, so this rises
    /// faster than the note count on a layered font, and it saturates at
    /// `get_maximum_polyphony` - past which a new note steals a sounding voice.
    pub fn get_active_voice_count(&self) -> usize {
        self.voices.active_voice_count
    }

    /// Gets the master volume.
    pub fn get_master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Sets the master volume.
    ///
    /// # Arguments
    ///
    /// * `value` - The new value of the master volume.
    pub fn set_master_volume(&mut self, value: f32) {
        self.master_volume = value;
    }
}

#[derive(Debug)]
struct Effects {
    reverb: Reverb,
    reverb_input: Vec<f32>,
    reverb_output_left: Vec<f32>,
    reverb_output_right: Vec<f32>,

    chorus: Chorus,
    chorus_input_left: Vec<f32>,
    chorus_input_right: Vec<f32>,
    chorus_output_left: Vec<f32>,
    chorus_output_right: Vec<f32>,
}

impl Effects {
    fn new(settings: &SynthesizerSettings) -> Effects {
        Self {
            reverb: Reverb::new(settings.sample_rate),
            reverb_input: vec![0_f32; settings.block_size],
            reverb_output_left: vec![0_f32; settings.block_size],
            reverb_output_right: vec![0_f32; settings.block_size],
            chorus: Chorus::new(settings.sample_rate, 0.002, 0.0019, 0.4),
            chorus_input_left: vec![0_f32; settings.block_size],
            chorus_input_right: vec![0_f32; settings.block_size],
            chorus_output_left: vec![0_f32; settings.block_size],
            chorus_output_right: vec![0_f32; settings.block_size],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundfont::SoundFont;
    use std::{fs::File, path::PathBuf};

    /// The modulator fixture is a complete font - one preset at bank 0/patch 0,
    /// one instrument, one region carrying no key or velocity range and so
    /// covering all of both - which means every note-on starts exactly one
    /// voice and voice bookkeeping can be asserted directly, with no rendering
    /// and none of the gitignored assets.
    fn test_synthesizer() -> Synthesizer {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("samples")
            .join("test_modulators.sf2");
        let mut file = File::open(&path).unwrap();
        let sound_font = Arc::new(SoundFont::new(&mut file).unwrap());
        Synthesizer::new(&sound_font, &SynthesizerSettings::new(44100)).unwrap()
    }

    /// The keys still sounding on a channel. A voice that has been ended is
    /// still active until its release envelope finishes, so this deliberately
    /// filters on `is_playing` rather than on membership.
    fn playing_keys(synthesizer: &mut Synthesizer, channel: i32) -> Vec<i32> {
        let mut keys: Vec<i32> = synthesizer
            .voices
            .get_active_voices()
            .iter()
            .filter(|voice| voice.channel() == channel && voice.is_playing())
            .map(|voice| voice.key())
            .collect();
        keys.sort_unstable();
        keys
    }

    const MONO_ON: i32 = 0x7E;
    const POLY_ON: i32 = 0x7F;

    /// The reported bug, with the key pair that produced it: Ebony and Ivory
    /// slurs a banjo line on a channel it has put into mono mode, so key 54
    /// arrives while key 52 is still sounding. Polyphonically that is a
    /// whole-tone dyad ringing for the best part of a second.
    #[test]
    fn mono_mode_stops_the_note_already_sounding() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);

        assert_eq!(playing_keys(&mut synthesizer, 0), vec![54]);
        // The stopped voice is still allocated - it is releasing, not gone.
        assert_eq!(synthesizer.voices.active_voice_count, 2);
    }

    /// The control: without CC126 both notes go on sounding, which is what the
    /// synthesizer did for every channel before this change.
    #[test]
    fn poly_mode_leaves_both_notes_sounding() {
        let mut synthesizer = test_synthesizer();

        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);

        assert_eq!(playing_keys(&mut synthesizer, 0), vec![52, 54]);
    }

    #[test]
    fn poly_mode_on_restores_polyphony() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);
        synthesizer.note_on(0, 52, 100);

        synthesizer.process_midi_message(0, 0xB0, POLY_ON, 0);
        // CC127 carries an implicit All Notes Off, so nothing is left from
        // before it.
        assert!(playing_keys(&mut synthesizer, 0).is_empty());

        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![52, 54]);
    }

    /// The MIDI spec makes every one of the four mode messages act as All
    /// Notes Off, and it is per channel.
    #[test]
    fn every_mode_message_acts_as_all_notes_off() {
        for cc in [0x7C, 0x7D, 0x7E, 0x7F] {
            let mut synthesizer = test_synthesizer();
            synthesizer.note_on(0, 60, 100);
            synthesizer.note_on(1, 60, 100);

            synthesizer.process_midi_message(0, 0xB0, cc, 0);

            assert!(
                playing_keys(&mut synthesizer, 0).is_empty(),
                "CC{cc} left channel 0 sounding"
            );
            assert_eq!(
                playing_keys(&mut synthesizer, 1),
                vec![60],
                "CC{cc} reached channel 1"
            );
        }
    }

    #[test]
    fn mono_mode_is_per_channel() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);
        synthesizer.note_on(1, 52, 100);
        synthesizer.note_on(1, 54, 100);

        assert_eq!(playing_keys(&mut synthesizer, 0), vec![54]);
        assert_eq!(playing_keys(&mut synthesizer, 1), vec![52, 54]);
    }

    /// Omni and mono/poly are orthogonal bits of the MIDI mode, so Omni Off/On
    /// must not select either one. Collapsing the four arms into a single
    /// `0x7C..=0x7F` that sets the mode is the tempting simplification this
    /// guards against - it would break the conventional CC124 + CC126 pair.
    #[test]
    fn omni_messages_do_not_change_mono_or_poly() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);
        synthesizer.process_midi_message(0, 0xB0, 0x7C, 0);
        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![54], "still mono");

        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, POLY_ON, 0);
        synthesizer.process_midi_message(0, 0xB0, 0x7D, 0);
        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);
        assert_eq!(
            playing_keys(&mut synthesizer, 0),
            vec![52, 54],
            "still poly"
        );
    }

    /// Files use running-status note-on-with-velocity-0 as note-off constantly.
    /// If the mono stop sat above the `velocity == 0` early return in
    /// `note_on`, every note-off would silence the channel and allocate
    /// nothing - so assert both the silence and the voice count.
    #[test]
    fn mono_mode_does_not_swallow_a_running_status_note_off() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.process_midi_message(0, 0x90, 60, 100);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![60]);

        synthesizer.process_midi_message(0, 0x90, 60, 0);
        assert!(playing_keys(&mut synthesizer, 0).is_empty());
        assert_eq!(synthesizer.voices.active_voice_count, 1);
    }

    /// Last-note priority. The exact shape Ebony and Ivory uses: a short grace
    /// note nested inside a long sustained one. Releasing the grace note has
    /// to hand the channel back to the note still held, or the sustained note
    /// is lost for the rest of its length - measured at 13 to 24 dB down
    /// across the 800 ms it should have been ringing.
    #[test]
    fn releasing_the_top_note_falls_back_to_the_one_still_held() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 76, 100);
        synthesizer.note_on(0, 78, 90);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![78]);

        synthesizer.note_off(0, 78);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![76]);

        // And releasing the last held key leaves the channel silent.
        synthesizer.note_off(0, 76);
        assert!(playing_keys(&mut synthesizer, 0).is_empty());
    }

    /// Releasing a key that is *not* the one sounding only forgets it. The
    /// channel goes on sounding the newer note, and when that is released it
    /// falls back past the forgotten key.
    #[test]
    fn releasing_a_key_that_is_not_sounding_changes_nothing_audible() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 60, 100);
        synthesizer.note_on(0, 64, 100);
        synthesizer.note_on(0, 67, 100);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![67]);

        synthesizer.note_off(0, 64);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![67]);

        synthesizer.note_off(0, 67);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![60]);
    }

    /// A key struck twice without its note-off must not end up on the stack
    /// twice, or releasing it once would fall back to itself and leave a voice
    /// sounding forever.
    #[test]
    fn a_repeated_key_does_not_fall_back_to_itself() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 60, 100);
        synthesizer.note_on(0, 60, 100);
        synthesizer.note_off(0, 60);

        assert!(playing_keys(&mut synthesizer, 0).is_empty());
    }

    /// All Notes Off has to drop the held keys with the voices, or the next
    /// note-off would resurrect one of them.
    #[test]
    fn all_notes_off_leaves_nothing_to_fall_back_to() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.note_on(0, 60, 100);
        synthesizer.note_on(0, 64, 100);
        synthesizer.process_midi_message(0, 0xB0, 0x7B, 0); // All Notes Off

        synthesizer.note_off(0, 64);
        assert!(playing_keys(&mut synthesizer, 0).is_empty());
    }

    /// The stack is bounded, and overflowing it drops the oldest key rather
    /// than panicking or refusing the newest note.
    #[test]
    fn the_held_key_stack_is_bounded() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        for key in 40..40 + 40 {
            synthesizer.note_on(0, key, 100);
        }
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![79]);

        // Sixteen keys are remembered, so falling back reaches 64 and no
        // further.
        for key in (40..40 + 40).rev() {
            synthesizer.note_off(0, key);
        }
        assert!(playing_keys(&mut synthesizer, 0).is_empty());
    }

    #[test]
    fn reset_returns_every_channel_to_poly() {
        let mut synthesizer = test_synthesizer();
        synthesizer.process_midi_message(0, 0xB0, MONO_ON, 1);

        synthesizer.reset();

        synthesizer.note_on(0, 52, 100);
        synthesizer.note_on(0, 54, 100);
        assert_eq!(playing_keys(&mut synthesizer, 0), vec![52, 54]);
    }
}
