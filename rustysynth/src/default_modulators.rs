#![allow(dead_code)]

use crate::generator_type::GeneratorType;
use crate::modulator::Modulator;
use crate::modulator_source::ModulatorSource;

const fn modulator(
    source: ModulatorSource,
    destination: u16,
    amount: i16,
    amount_source: ModulatorSource,
) -> Modulator {
    Modulator {
        source,
        destination,
        amount,
        amount_source,
        transform: 0,
    }
}

const NONE: ModulatorSource = ModulatorSource::general(
    ModulatorSource::NO_CONTROLLER,
    ModulatorSource::CURVE_LINEAR,
    false,
    false,
);

/// The SF2 default modulators, which apply to every instrument region unless
/// the font replaces one by shipping a modulator with the same source and
/// destination.
///
/// This is the spec's set of ten with two deliberate omissions and two
/// deliberate deviations, all of them recorded in CHANGELOG.md:
///
/// Default 2, velocity to filter cutoff at -2400 cents, is **omitted**,
/// matching FluidSynth, which hard-codes it away. Shipping it would darken
/// soft notes on every SoundFont, including fonts that were voiced without it.
///
/// Default 10, pitch wheel to fine tune, is **omitted here and handled
/// natively** in `Voice::process`, which multiplies the pitch bend by the RPN 0
/// range directly. Routing it through the modulator engine would give
/// `12700 * (sensitivity / 128)`, or 198.4 cents at a range of 2 semitones,
/// making full deflection 1.6 cents flat. It would also not work: the
/// oscillator latches its tune at note-on, so a dynamic modulator on
/// `fineTune` would never bend.
///
/// The reverb and chorus sends use amount 1000 rather than the spec's 200, so
/// that CC91 and CC93 keep the 0..100% range this crate has always given them.
/// A font shipping its own send modulators overrides this, which is why
/// `SynthesizerSettings` also carries a send scale.
pub(crate) const DEFAULT_MODULATORS: [Modulator; 8] = [
    // 1. Note-on velocity to initial attenuation.
    modulator(
        ModulatorSource::general(
            ModulatorSource::NOTE_ON_VELOCITY,
            ModulatorSource::CURVE_CONCAVE,
            false,
            true,
        ),
        GeneratorType::INITIAL_ATTENUATION,
        960,
        NONE,
    ),
    // 3. Channel pressure to vibrato depth. New capability: this crate
    //    previously ignored channel pressure entirely.
    modulator(
        ModulatorSource::general(
            ModulatorSource::CHANNEL_PRESSURE,
            ModulatorSource::CURVE_LINEAR,
            false,
            false,
        ),
        GeneratorType::VIBRATO_LFO_TO_PITCH,
        50,
        NONE,
    ),
    // 4. CC1 modulation wheel to vibrato depth.
    modulator(
        ModulatorSource::cc(1, ModulatorSource::CURVE_LINEAR, false, false),
        GeneratorType::VIBRATO_LFO_TO_PITCH,
        50,
        NONE,
    ),
    // 5. CC7 channel volume to initial attenuation.
    modulator(
        ModulatorSource::cc(7, ModulatorSource::CURVE_CONCAVE, false, true),
        GeneratorType::INITIAL_ATTENUATION,
        960,
        NONE,
    ),
    // 6. CC10 pan. The spec says 1000; 500 is what gives the -50..50 range
    //    this crate uses, and matches FluidSynth.
    modulator(
        ModulatorSource::cc(10, ModulatorSource::CURVE_LINEAR, true, false),
        GeneratorType::PAN,
        500,
        NONE,
    ),
    // 7. CC11 expression to initial attenuation.
    modulator(
        ModulatorSource::cc(11, ModulatorSource::CURVE_CONCAVE, false, true),
        GeneratorType::INITIAL_ATTENUATION,
        960,
        NONE,
    ),
    // 8. CC91 reverb send. Amount 1000, not the spec's 200.
    modulator(
        ModulatorSource::cc(91, ModulatorSource::CURVE_LINEAR, false, false),
        GeneratorType::REVERB_EFFECTS_SEND,
        1000,
        NONE,
    ),
    // 9. CC93 chorus send. Amount 1000, not the spec's 200.
    modulator(
        ModulatorSource::cc(93, ModulatorSource::CURVE_LINEAR, false, false),
        GeneratorType::CHORUS_EFFECTS_SEND,
        1000,
        NONE,
    ),
];
