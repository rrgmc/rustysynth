#![allow(dead_code)]

use crate::loop_mode::LoopMode;
use crate::synthesizer_settings::SynthesizerSettings;

// In this class, fixed-point numbers are used for speed-up.
// A fixed-point number is expressed by Int64, whose lower 24 bits represent the fraction part,
// and the rest represent the integer part.
// For clarity, fixed-point number variables have a suffix "_fp".

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct Oscillator {
    synthesizer_sample_rate: i32,

    loop_mode: LoopMode,
    sample_sample_rate: i32,
    start: i32,
    end: i32,
    start_loop: i32,
    end_loop: i32,
    root_key: i32,
    key: i32,

    tune: f32,
    pitch_change_scale: f32,
    sample_rate_ratio: f32,

    looping: bool,

    position_fp: i64,
}

impl Oscillator {
    const FRAC_BITS: i32 = 24;
    const FRAC_UNIT: i64 = 1_i64 << Oscillator::FRAC_BITS;
    const FP_TO_SAMPLE: f32 = 1_f32 / (32768 * Oscillator::FRAC_UNIT) as f32;

    pub(crate) fn new(settings: &SynthesizerSettings) -> Self {
        Self {
            synthesizer_sample_rate: settings.sample_rate,
            loop_mode: LoopMode::NoLoop,
            sample_sample_rate: 0,
            start: 0,
            end: 0,
            start_loop: 0,
            end_loop: 0,
            root_key: 0,
            key: 0,
            tune: 0_f32,
            pitch_change_scale: 0_f32,
            sample_rate_ratio: 0_f32,
            looping: false,
            position_fp: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        &mut self,
        loop_mode: LoopMode,
        sample_rate: i32,
        start: i32,
        end: i32,
        start_loop: i32,
        end_loop: i32,
        root_key: i32,
        key: i32,
        coarse_tune: i32,
        fine_tune: i32,
        scale_tuning: i32,
    ) {
        self.loop_mode = loop_mode;
        self.sample_sample_rate = sample_rate;
        self.start = start;
        self.end = end;
        self.start_loop = start_loop;
        self.end_loop = end_loop;
        self.root_key = root_key;
        self.key = key;

        self.tune = coarse_tune as f32 + 0.01_f32 * fine_tune as f32;
        self.pitch_change_scale = 0.01_f32 * scale_tuning as f32;
        self.sample_rate_ratio = sample_rate as f32 / self.synthesizer_sample_rate as f32;
        self.looping = self.loop_mode != LoopMode::NoLoop;
        self.position_fp = (start as i64) << Oscillator::FRAC_BITS;
    }

    pub(crate) fn release(&mut self) {
        if self.loop_mode == LoopMode::LoopUntilNoteOff {
            self.looping = false;
        }
    }

    /// `pitch` is the note's key plus everything modulating it this block -
    /// the two LFOs, the modulation envelope, the channel tune and the pitch
    /// bend.
    ///
    /// Only the key-to-root interval is scaled by `scaleTuning`. SF2 2.04
    /// 8.1.2 defines that generator as "the degree to which MIDI key number
    /// influences pitch", which is the interval and nothing else; the
    /// modulation is in real semitones and is added as such. Scaling it too -
    /// which is what this did while it followed MeltySynth - makes a region at
    /// `scaleTuning` 0 swallow pitch bend and vibrato whole, and GeneralUser-GS
    /// ships eight such regions to SGM-V2.01's 174.
    pub(crate) fn process(&mut self, data: &[i16], block: &mut [f32], pitch: f32) -> bool {
        let pitch_ratio = self.pitch_ratio(pitch);
        self.fill_block(data, block, pitch_ratio as f64)
    }

    fn pitch_ratio(&self, pitch: f32) -> f32 {
        let interval = (self.key - self.root_key) as f32;
        let modulation = pitch - self.key as f32;
        let pitch_change = self.pitch_change_scale * interval + modulation + self.tune;
        self.sample_rate_ratio * 2_f32.powf(pitch_change / 12_f32)
    }

    fn fill_block(&mut self, data: &[i16], block: &mut [f32], pitch_ratio: f64) -> bool {
        let pitch_ratio_fp = (Oscillator::FRAC_UNIT as f64 * pitch_ratio) as i64;

        if self.looping {
            self.fill_block_continuous(data, block, pitch_ratio_fp)
        } else {
            self.fill_block_no_loop(data, block, pitch_ratio_fp)
        }
    }

    fn fill_block_no_loop(&mut self, data: &[i16], block: &mut [f32], pitch_ratio_fp: i64) -> bool {
        for t in 0..block.len() {
            let index = (self.position_fp >> Oscillator::FRAC_BITS) as usize;
            if index >= self.end as usize {
                if t > 0 {
                    let len = block.len();
                    block[t..len].fill(0_f32);
                    return true;
                } else {
                    return false;
                }
            }

            let x1 = data[index] as i64;
            let x2 = data[index + 1] as i64;
            let a_fp = self.position_fp & (Oscillator::FRAC_UNIT - 1);
            block[t] = Oscillator::FP_TO_SAMPLE
                * ((x1 << Oscillator::FRAC_BITS) + a_fp * (x2 - x1)) as f32;

            self.position_fp += pitch_ratio_fp;
        }

        true
    }

    fn fill_block_continuous(
        &mut self,
        data: &[i16],
        block: &mut [f32],
        pitch_ratio_fp: i64,
    ) -> bool {
        let end_loop_fp = (self.end_loop as i64) << Oscillator::FRAC_BITS;
        let loop_length = (self.end_loop - self.start_loop) as i64;
        let loop_length_fp = loop_length << Oscillator::FRAC_BITS;

        for sample in block.iter_mut() {
            // `while`, not `if`: one subtraction per output sample only brings
            // the position back inside the loop while the pitch ratio is below
            // the loop length. Above it - a short loop played far above its
            // root key - the position walks past `end_loop` and keeps going,
            // and nothing here bounds-checks, so it reads a neighbouring
            // sample's audio and eventually indexes off the end of the wave
            // data. `drop_unplayable_regions` guarantees `start_loop <
            // end_loop` for a looping region, so `loop_length_fp` is at least
            // one unit and this terminates.
            while self.position_fp >= end_loop_fp {
                self.position_fp -= loop_length_fp;
            }

            let index1 = (self.position_fp >> Oscillator::FRAC_BITS) as usize;
            let mut index2 = index1 + 1;
            if index2 >= self.end_loop as usize {
                index2 -= loop_length as usize;
            }

            let x1 = data[index1] as i64;
            let x2 = data[index2] as i64;
            let a_fp = self.position_fp & (Oscillator::FRAC_UNIT - 1);
            *sample = Oscillator::FP_TO_SAMPLE
                * ((x1 << Oscillator::FRAC_BITS) + a_fp * (x2 - x1)) as f32;

            self.position_fp += pitch_ratio_fp;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An oscillator playing `key` from a sample rooted at `root_key`, at the
    /// synthesizer's own rate so `sample_rate_ratio` is 1 and the ratio reads
    /// directly as a transposition.
    fn oscillator(scale_tuning: i32, root_key: i32, key: i32) -> Oscillator {
        let settings = SynthesizerSettings::new(44100);
        let mut oscillator = Oscillator::new(&settings);
        oscillator.start(
            LoopMode::NoLoop,
            44100,
            0,
            1000,
            0,
            1000,
            root_key,
            key,
            0,
            0,
            scale_tuning,
        );
        oscillator
    }

    fn semitones(ratio: f32) -> f32 {
        12_f32 * ratio.log2()
    }

    #[test]
    fn scale_tuning_bends_the_key_interval() {
        // What the generator is for: at 50 cents per key, an octave above the
        // root sounds a perfect fourth above it instead.
        let oscillator = oscillator(50, 60, 72);
        assert!((semitones(oscillator.pitch_ratio(72_f32)) - 6_f32).abs() < 1e-4);
    }

    #[test]
    fn scale_tuning_leaves_the_modulation_alone() {
        // A region at scaleTuning 0 is a fixed-pitch region - every key plays
        // the sample untransposed - but pitch bend and vibrato still have to
        // reach it. Scaling them too made a whole class of region, including
        // eight in GeneralUser-GS and 174 in SGM-V2.01, silently deaf to the
        // pitch wheel.
        let oscillator = oscillator(0, 60, 72);

        assert!(semitones(oscillator.pitch_ratio(72_f32)).abs() < 1e-4);
        assert!((semitones(oscillator.pitch_ratio(74_f32)) - 2_f32).abs() < 1e-4);
        assert!((semitones(oscillator.pitch_ratio(71.5_f32)) + 0.5_f32).abs() < 1e-4);
    }

    #[test]
    fn a_normal_region_is_unchanged() {
        // scaleTuning 100 is the default and by far the common case, and there
        // the split has to be a no-op: the interval and the modulation are
        // both scaled by one.
        let oscillator = oscillator(100, 60, 72);

        assert!((semitones(oscillator.pitch_ratio(72_f32)) - 12_f32).abs() < 1e-4);
        assert!((semitones(oscillator.pitch_ratio(74_f32)) - 14_f32).abs() < 1e-4);
    }
}
