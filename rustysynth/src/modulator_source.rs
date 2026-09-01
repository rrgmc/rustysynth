#![allow(dead_code)]

use crate::channel::Channel;
use crate::soundfont_math::SoundFontMath;

/// The source of a modulator: which controller drives it, and how that
/// controller's value is shaped before it is applied.
///
/// This is the decoded form of the SF2 `SFModulator` bit field, which packs
/// the controller index, the controller palette, the direction, the polarity
/// and the curve type into one `u16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModulatorSource {
    /// Controller index: a MIDI CC number when `is_cc`, otherwise an entry in
    /// the general controller palette.
    pub(crate) index: u8,
    /// True when `index` is a MIDI continuous controller rather than a general
    /// controller.
    pub(crate) is_cc: bool,
    /// True when the source runs from maximum to minimum rather than minimum
    /// to maximum.
    pub(crate) is_negative: bool,
    /// True when the source spans -1..1 rather than 0..1.
    pub(crate) is_bipolar: bool,
    /// Curve type: 0 linear, 1 concave, 2 convex, 3 switch.
    pub(crate) curve: u8,
}

impl ModulatorSource {
    // The general controller palette, used when `is_cc` is false.
    pub(crate) const NO_CONTROLLER: u8 = 0;
    pub(crate) const NOTE_ON_VELOCITY: u8 = 2;
    pub(crate) const NOTE_ON_KEY_NUMBER: u8 = 3;
    pub(crate) const POLY_PRESSURE: u8 = 10;
    pub(crate) const CHANNEL_PRESSURE: u8 = 13;
    pub(crate) const PITCH_WHEEL: u8 = 14;
    pub(crate) const PITCH_WHEEL_SENSITIVITY: u8 = 16;
    pub(crate) const LINK: u8 = 127;

    pub(crate) const CURVE_LINEAR: u8 = 0;
    pub(crate) const CURVE_CONCAVE: u8 = 1;
    pub(crate) const CURVE_CONVEX: u8 = 2;
    pub(crate) const CURVE_SWITCH: u8 = 3;

    /// The slope of the concave and convex curves.
    ///
    /// The SF2 curves are defined so that a source at full scale through a
    /// 960 cB modulator gives 96 dB of attenuation, which works out to
    /// `1 + (5/12) * log10(v)`. That constant is what makes the default
    /// velocity modulator reproduce `2 * linear_to_decibels(velocity / 127)`
    /// exactly - see the unit tests.
    const CURVE_SLOPE: f32 = 5_f32 / 12_f32;

    pub(crate) fn from_bits(bits: u16) -> Self {
        Self {
            index: (bits & 0x7F) as u8,
            is_cc: (bits & 0x0080) != 0,
            is_negative: (bits & 0x0100) != 0,
            is_bipolar: (bits & 0x0200) != 0,
            curve: ((bits >> 10) & 0x3F) as u8,
        }
    }

    pub(crate) fn to_bits(self) -> u16 {
        (self.index as u16 & 0x7F)
            | if self.is_cc { 0x0080 } else { 0 }
            | if self.is_negative { 0x0100 } else { 0 }
            | if self.is_bipolar { 0x0200 } else { 0 }
            | ((self.curve as u16 & 0x3F) << 10)
    }

    /// Builds a source from the general controller palette. Used by the
    /// default modulator table.
    pub(crate) const fn general(index: u8, curve: u8, is_bipolar: bool, is_negative: bool) -> Self {
        Self {
            index,
            is_cc: false,
            is_negative,
            is_bipolar,
            curve,
        }
    }

    /// Builds a source from a MIDI continuous controller. Used by the default
    /// modulator table.
    pub(crate) const fn cc(index: u8, curve: u8, is_bipolar: bool, is_negative: bool) -> Self {
        Self {
            index,
            is_cc: true,
            is_negative,
            is_bipolar,
            curve,
        }
    }

    /// A linked source, whose value comes from another modulator's output.
    /// Not supported; modulators using one are dropped at load time.
    pub(crate) fn is_link(&self) -> bool {
        !self.is_cc && self.index == ModulatorSource::LINK
    }

    pub(crate) fn is_no_controller(&self) -> bool {
        !self.is_cc && self.index == ModulatorSource::NO_CONTROLLER
    }

    /// True when the source cannot change over the life of a voice, so a
    /// modulator using it only has to be evaluated at note-on.
    pub(crate) fn is_static(&self) -> bool {
        !self.is_cc
            && matches!(
                self.index,
                ModulatorSource::NO_CONTROLLER
                    | ModulatorSource::NOTE_ON_VELOCITY
                    | ModulatorSource::NOTE_ON_KEY_NUMBER
            )
    }

    /// True when the source is one SF2 2.04 section 8.2.1 forbids: bank
    /// select, data entry, and the RPN/NRPN selectors, all of which carry
    /// parameter numbers rather than continuous values.
    pub(crate) fn is_illegal_cc(&self) -> bool {
        self.is_cc && matches!(self.index, 0 | 6 | 32 | 38 | 98..=101)
    }

    /// True when the source names a controller this build can read at all.
    pub(crate) fn is_known(&self) -> bool {
        if self.is_cc {
            return true;
        }

        matches!(
            self.index,
            ModulatorSource::NO_CONTROLLER
                | ModulatorSource::NOTE_ON_VELOCITY
                | ModulatorSource::NOTE_ON_KEY_NUMBER
                | ModulatorSource::POLY_PRESSURE
                | ModulatorSource::CHANNEL_PRESSURE
                | ModulatorSource::PITCH_WHEEL
                | ModulatorSource::PITCH_WHEEL_SENSITIVITY
        )
    }

    /// The controller's current value, normalized to 0..1.
    ///
    /// Normalization deliberately uses RustySynth's own ranges - 127 for 7-bit
    /// sources and 16383 for the four controllers `Channel` tracks at 14 bits -
    /// rather than FluidSynth's 128 and 16384. Matching `Channel`'s existing
    /// getters is what lets the default modulators reproduce the hardcoded
    /// paths they replace; FluidSynth's divisors would shift velocity 64 by
    /// 0.14 dB.
    pub(crate) fn raw_value(&self, channel: &Channel, key: i32, velocity: i32) -> f32 {
        if self.is_cc {
            return match self.index {
                // The four controllers tracked at full 14-bit resolution.
                1 => channel.get_modulation_normalized(),
                7 => channel.get_volume(),
                10 => channel.get_pan_normalized(),
                11 => channel.get_expression(),
                index => (1_f32 / 127_f32) * channel.get_cc(index) as f32,
            };
        }

        match self.index {
            ModulatorSource::NOTE_ON_VELOCITY => (1_f32 / 127_f32) * velocity as f32,
            ModulatorSource::NOTE_ON_KEY_NUMBER => (1_f32 / 127_f32) * key as f32,
            ModulatorSource::POLY_PRESSURE => {
                (1_f32 / 127_f32) * channel.get_poly_pressure(key) as f32
            }
            ModulatorSource::CHANNEL_PRESSURE => {
                (1_f32 / 127_f32) * channel.get_channel_pressure() as f32
            }
            ModulatorSource::PITCH_WHEEL => channel.get_pitch_bend_normalized(),
            ModulatorSource::PITCH_WHEEL_SENSITIVITY => {
                (1_f32 / 127_f32) * channel.get_pitch_bend_range()
            }
            // No Controller, and anything unrecognized, contribute unity so
            // that the modulator reduces to its amount.
            _ => 1_f32,
        }
    }

    /// The SF2 concave curve over a normalized input.
    ///
    /// Evaluated in closed form rather than through a table. FluidSynth can
    /// use a 128-entry table because all of its sources are 7-bit and land
    /// exactly on entries. RustySynth tracks four controllers at 14 bits, which
    /// land between entries, and there the same table carries up to 6 dB of
    /// interpolation error at the corner where the curve clamps.
    pub(crate) fn concave(v: f32) -> f32 {
        if v <= 0_f32 {
            return 0_f32;
        }
        if v >= 1_f32 {
            return 1_f32;
        }

        let value = 1_f32 + ModulatorSource::CURVE_SLOPE * v.log10();

        if value > 0_f32 {
            value
        } else {
            0_f32
        }
    }

    /// The SF2 convex curve, the mirror image of the concave one.
    pub(crate) fn convex(v: f32) -> f32 {
        1_f32 - ModulatorSource::concave(1_f32 - v)
    }

    fn curve_positive(&self, v: f32) -> f32 {
        match self.curve {
            ModulatorSource::CURVE_CONCAVE => ModulatorSource::concave(v),
            ModulatorSource::CURVE_CONVEX => ModulatorSource::convex(v),
            ModulatorSource::CURVE_SWITCH => {
                if v >= 0.5_f32 {
                    1_f32
                } else {
                    0_f32
                }
            }
            // Linear, and any curve type the spec does not define.
            _ => v,
        }
    }

    /// Shapes the controller's current value into the factor the modulator's
    /// amount is multiplied by.
    ///
    /// A negative direction inverts the curve's *output*, not its input. That
    /// distinction matters: running `1 - v` through a concave curve instead
    /// gives 90 dB of attenuation at velocity 64, which is silence.
    pub(crate) fn transform(&self, channel: &Channel, key: i32, velocity: i32) -> f32 {
        // No Controller has no curve; the spec defines its output as unity.
        if self.is_no_controller() {
            return 1_f32;
        }

        // A NaN here would propagate into the biquad coefficients and from
        // there into the reverb and chorus state, which never recovers, so it
        // is folded into the zero branch rather than clamped.
        let raw = self.raw_value(channel, key, velocity);
        let v = if raw.is_nan() {
            0_f32
        } else {
            SoundFontMath::clamp(raw, 0_f32, 1_f32)
        };

        if self.is_bipolar {
            let bipolar = 2_f32 * v - 1_f32;
            let shaped = self.curve_positive(bipolar.abs());
            let signed = if bipolar < 0_f32 { -shaped } else { shaped };

            if self.is_negative {
                -signed
            } else {
                signed
            }
        } else {
            let shaped = self.curve_positive(v);

            if self.is_negative {
                1_f32 - shaped
            } else {
                shaped
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Channel {
        Channel::new(false)
    }

    /// The velocity to attenuation default, as SF2 defines it.
    fn velocity_to_attenuation() -> ModulatorSource {
        ModulatorSource::general(
            ModulatorSource::NOTE_ON_VELOCITY,
            ModulatorSource::CURVE_CONCAVE,
            false,
            true,
        )
    }

    #[test]
    fn bit_field_decodes_the_spec_example() {
        // The velocity source the SF2 spec writes as 0x0502.
        let source = ModulatorSource::from_bits(0x0502);
        assert_eq!(source.index, ModulatorSource::NOTE_ON_VELOCITY);
        assert!(!source.is_cc);
        assert!(source.is_negative);
        assert!(!source.is_bipolar);
        assert_eq!(source.curve, ModulatorSource::CURVE_CONCAVE);

        for bits in [0x0002_u16, 0x0582, 0x020E, 0x00DB, 0x0100, 0xFFFF] {
            let round_tripped = ModulatorSource::from_bits(bits).to_bits();
            assert_eq!(round_tripped, bits, "0x{bits:04X} did not round trip");
        }
    }

    /// The whole design rests on the SF2 default modulators being what
    /// RustySynth already computes, written differently. If this drifts, the
    /// defaults are miscalibrated and every rendered file changes.
    #[test]
    fn velocity_default_reproduces_the_legacy_attenuation_curve() {
        let channel = channel();
        let source = velocity_to_attenuation();

        for velocity in 1..=127 {
            // What voice.rs computed before modulators existed.
            let legacy_db = 2_f32 * SoundFontMath::linear_to_decibels(velocity as f32 / 127_f32);

            // What the default modulator computes: 960 cB through a concave,
            // negative, unipolar velocity source, read as attenuation.
            let modulator_db = -0.1_f32 * 960_f32 * source.transform(&channel, 60, velocity);

            assert!(
                (legacy_db - modulator_db).abs() < 1.0e-4,
                "velocity {velocity}: legacy {legacy_db} dB, modulator {modulator_db} dB"
            );
        }
    }

    /// Same check for the two controllers that become concave attenuation
    /// modulators, across the 14-bit range this crate tracks them at.
    #[test]
    fn volume_and_expression_defaults_reproduce_the_legacy_channel_gain() {
        let volume = ModulatorSource::cc(7, ModulatorSource::CURVE_CONCAVE, false, true);
        let expression = ModulatorSource::cc(11, ModulatorSource::CURVE_CONCAVE, false, true);

        for coarse in 1..=127 {
            let mut channel = channel();
            channel.set_volume_coarse(coarse);
            channel.set_expression_coarse(128 - coarse);

            // Legacy: channel_gain = (volume * expression)^2, voice.rs:240.
            let legacy_gain = {
                let ve = channel.get_volume() * channel.get_expression();
                ve * ve
            };

            // Modulator: two concave 960 cB attenuations, summed in cB.
            let attenuation_cb = 960_f32
                * (volume.transform(&channel, 60, 100) + expression.transform(&channel, 60, 100));
            let modulator_gain = SoundFontMath::decibels_to_linear(-0.1_f32 * attenuation_cb);

            assert!(
                (legacy_gain - modulator_gain).abs() < 1.0e-5,
                "cc7 {coarse}: legacy {legacy_gain}, modulator {modulator_gain}"
            );
        }
    }

    /// CC10 at amount 500 has to land on `Channel::get_pan`, which spans
    /// -50..50.
    #[test]
    fn pan_default_reproduces_the_legacy_pan() {
        let source = ModulatorSource::cc(10, ModulatorSource::CURVE_LINEAR, true, false);

        for coarse in 0..=127 {
            let mut channel = channel();
            channel.set_pan_coarse(coarse);

            let legacy = channel.get_pan();
            let modulated = 0.1_f32 * 500_f32 * source.transform(&channel, 60, 100);

            assert!(
                (legacy - modulated).abs() < 1.0e-3,
                "cc10 {coarse}: legacy {legacy}, modulator {modulated}"
            );
        }
    }

    /// CC1 at amount 50 has to land on the vibrato depth the legacy path took
    /// from `0.01 * get_modulation()`, in semitones.
    #[test]
    fn modulation_default_reproduces_the_legacy_vibrato_depth() {
        let source = ModulatorSource::cc(1, ModulatorSource::CURVE_LINEAR, false, false);

        for coarse in 0..=127 {
            let mut channel = channel();
            channel.set_modulation_coarse(coarse);

            let legacy = 0.01_f32 * channel.get_modulation();
            let modulated = 0.01_f32 * 50_f32 * source.transform(&channel, 60, 100);

            assert!(
                (legacy - modulated).abs() < 1.0e-5,
                "cc1 {coarse}: legacy {legacy}, modulator {modulated}"
            );
        }
    }

    /// CC91 at amount 1000 keeps the 0..100% send range this crate has always
    /// had, rather than the spec's 0..20%.
    #[test]
    fn reverb_send_default_reproduces_the_legacy_send() {
        let source = ModulatorSource::cc(91, ModulatorSource::CURVE_LINEAR, false, false);

        for value in 0..=127 {
            let mut channel = channel();
            channel.set_reverb_send(value);

            let legacy = channel.get_reverb_send();
            let modulated = 0.001_f32 * 1000_f32 * source.transform(&channel, 60, 100);

            assert!(
                (legacy - modulated).abs() < 1.0e-6,
                "cc91 {value}: legacy {legacy}, modulator {modulated}"
            );
        }
    }

    /// A negative direction inverts the curve output, not its input. Getting
    /// this backwards yields 90 dB of attenuation at velocity 64 - silence -
    /// and is the easiest way to break the whole feature.
    #[test]
    fn negative_direction_inverts_the_output() {
        let channel = channel();
        let negative = velocity_to_attenuation();
        let positive = ModulatorSource::general(
            ModulatorSource::NOTE_ON_VELOCITY,
            ModulatorSource::CURVE_CONCAVE,
            false,
            false,
        );

        for velocity in [1, 32, 64, 100, 127] {
            let n = negative.transform(&channel, 60, velocity);
            let p = positive.transform(&channel, 60, velocity);
            assert!(
                (n + p - 1_f32).abs() < 1.0e-6,
                "velocity {velocity}: negative {n} and positive {p} should sum to 1"
            );
        }

        // Full velocity means no attenuation; silence means full attenuation.
        assert!(negative.transform(&channel, 60, 127).abs() < 1.0e-6);
        assert!((negative.transform(&channel, 60, 0) - 1_f32).abs() < 1.0e-6);
    }

    #[test]
    fn curves_span_the_unit_square_and_mirror_each_other() {
        for i in 0..=100 {
            let v = i as f32 / 100_f32;
            let concave = ModulatorSource::concave(v);
            let convex = ModulatorSource::convex(v);

            assert!(
                (0_f32..=1_f32).contains(&concave),
                "concave({v}) = {concave}"
            );
            assert!((0_f32..=1_f32).contains(&convex), "convex({v}) = {convex}");

            let mirrored = 1_f32 - ModulatorSource::concave(1_f32 - v);
            assert!((convex - mirrored).abs() < 1.0e-6);
        }

        assert_eq!(ModulatorSource::concave(0_f32), 0_f32);
        assert_eq!(ModulatorSource::concave(1_f32), 1_f32);
        assert_eq!(ModulatorSource::convex(0_f32), 0_f32);
        assert_eq!(ModulatorSource::convex(1_f32), 1_f32);
    }

    #[test]
    fn no_controller_contributes_unity() {
        let channel = channel();
        let source = ModulatorSource::general(
            ModulatorSource::NO_CONTROLLER,
            ModulatorSource::CURVE_CONCAVE,
            true,
            true,
        );

        // Whatever the curve and direction say, the spec fixes the output at 1
        // so that the modulator reduces to its amount.
        assert_eq!(source.transform(&channel, 60, 100), 1_f32);
    }

    #[test]
    fn classification_rules() {
        assert!(ModulatorSource::general(ModulatorSource::LINK, 0, false, false).is_link());
        assert!(
            ModulatorSource::general(ModulatorSource::NOTE_ON_VELOCITY, 0, false, false)
                .is_static()
        );
        assert!(
            ModulatorSource::general(ModulatorSource::NOTE_ON_KEY_NUMBER, 0, false, false)
                .is_static()
        );
        assert!(!ModulatorSource::cc(7, 0, false, false).is_static());
        assert!(
            !ModulatorSource::general(ModulatorSource::CHANNEL_PRESSURE, 0, false, false)
                .is_static()
        );

        // Bank select, data entry and the RPN/NRPN selectors carry parameter
        // numbers, not continuous values.
        for index in [0, 6, 32, 38, 98, 99, 100, 101] {
            assert!(
                ModulatorSource::cc(index, 0, false, false).is_illegal_cc(),
                "cc{index} should be rejected"
            );
        }
        for index in [1, 7, 10, 11, 64, 74, 91, 93] {
            assert!(!ModulatorSource::cc(index, 0, false, false).is_illegal_cc());
        }

        // Unassigned general controller slots cannot be read.
        assert!(!ModulatorSource::general(5, 0, false, false).is_known());
        assert!(ModulatorSource::general(ModulatorSource::PITCH_WHEEL, 0, false, false).is_known());
    }
}
