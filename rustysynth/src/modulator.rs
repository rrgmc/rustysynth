#![allow(dead_code)]

use std::io::Read;

use crate::binary_reader::BinaryReader;
use crate::channel::Channel;
use crate::error::SoundFontError;
use crate::generator_type::GeneratorType;
use crate::modulator_source::ModulatorSource;

/// A SoundFont modulator: a real-time link from a controller to a synthesis
/// parameter.
///
/// Where a generator sets a parameter to a fixed value at note-on, a modulator
/// keeps adjusting it while the note sounds. The SF2 default modulator set is
/// what makes velocity affect loudness and CC7 act as a volume fader; a font
/// may add its own, or replace a default by shipping one with the same source
/// and destination.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Modulator {
    pub(crate) source: ModulatorSource,
    pub(crate) destination: u16,
    pub(crate) amount: i16,
    pub(crate) amount_source: ModulatorSource,
    pub(crate) transform: u16,
}

impl Modulator {
    /// The size of one `SFModList` record.
    const RECORD_SIZE: usize = 10;

    fn new<R: Read>(reader: &mut R) -> Result<Self, SoundFontError> {
        let source = ModulatorSource::from_bits(BinaryReader::read_u16(reader)?);
        let destination = BinaryReader::read_u16(reader)?;
        let amount = BinaryReader::read_i16(reader)?;
        let amount_source = ModulatorSource::from_bits(BinaryReader::read_u16(reader)?);
        let transform = BinaryReader::read_u16(reader)?;

        Ok(Self {
            source,
            destination,
            amount,
            amount_source,
            transform,
        })
    }

    pub(crate) fn read_from_chunk<R: Read>(
        reader: &mut R,
        size: usize,
    ) -> Result<Vec<Modulator>, SoundFontError> {
        if size == 0 || !size.is_multiple_of(Modulator::RECORD_SIZE) {
            return Err(SoundFontError::InvalidModulatorList);
        }

        // Unlike the generator lists, a modulator list holding nothing but its
        // terminator is both common and legal.
        let count = size / Modulator::RECORD_SIZE - 1;

        let mut modulators: Vec<Modulator> = Vec::new();
        for _i in 0..count {
            modulators.push(Modulator::new(reader)?);
        }

        // The last one is the terminator.
        Modulator::new(reader)?;

        Ok(modulators)
    }

    /// True when the modulator's output feeds another modulator rather than a
    /// generator. Not supported.
    fn is_linked(&self) -> bool {
        self.source.is_link() || self.amount_source.is_link() || (self.destination & 0x8000) != 0
    }

    /// True when the destination is a generator a modulator is allowed to
    /// drive.
    ///
    /// Sample addressing, the range and override generators, and
    /// `instrument` / `sampleID` all select *which* data plays rather than how
    /// it sounds, and SF2 2.04 section 8.1.2 forbids modulating them. Letting
    /// one through would move loop points at block rate.
    fn has_valid_destination(&self) -> bool {
        if self.destination as usize >= GeneratorType::COUNT {
            return false;
        }

        !matches!(
            self.destination,
            GeneratorType::START_ADDRESS_OFFSET
                | GeneratorType::END_ADDRESS_OFFSET
                | GeneratorType::START_LOOP_ADDRESS_OFFSET
                | GeneratorType::END_LOOP_ADDRESS_OFFSET
                | GeneratorType::START_ADDRESS_COARSE_OFFSET
                | GeneratorType::END_ADDRESS_COARSE_OFFSET
                | GeneratorType::INSTRUMENT
                | GeneratorType::KEY_RANGE
                | GeneratorType::VELOCITY_RANGE
                | GeneratorType::START_LOOP_ADDRESS_COARSE_OFFSET
                | GeneratorType::KEY_NUMBER
                | GeneratorType::VELOCITY
                | GeneratorType::END_LOOP_ADDRESS_COARSE_OFFSET
                | GeneratorType::SAMPLE_ID
                | GeneratorType::SAMPLE_MODES
                | GeneratorType::EXCLUSIVE_CLASS
                | GeneratorType::OVERRIDING_ROOT_KEY
        )
    }

    /// True when the modulator can be honored as written.
    ///
    /// A rejected modulator is dropped at load time rather than at note-on, so
    /// the audio thread never has to reason about it.
    ///
    /// `transform != 0` is rejected because preset and instrument modulator
    /// amounts are summed at voice start, which is only equivalent to
    /// evaluating them separately while the transform is linear.
    /// `amount * f + amount' * f == (amount + amount') * f` does not hold for
    /// the absolute-value transform. SF2 2.04 section 8.4 defines only
    /// transform 0, and no font in the test set uses another.
    pub(crate) fn is_supported(&self) -> bool {
        !self.is_linked()
            && self.transform == 0
            && self.has_valid_destination()
            && self.source.is_known()
            && self.amount_source.is_known()
            && !self.source.is_illegal_cc()
            && !self.amount_source.is_illegal_cc()
    }

    /// True when two modulators address the same thing, and so one replaces
    /// the other rather than adding to it.
    ///
    /// Per SF2 2.04 section 9.5.4 the comparison covers the source, the
    /// destination and the amount source, but deliberately *not* the amount or
    /// the transform - replacing the amount is the whole point of an override.
    pub(crate) fn is_identical(&self, other: &Modulator) -> bool {
        self.source == other.source
            && self.destination == other.destination
            && self.amount_source == other.amount_source
    }

    /// True when neither source can change while the voice sounds, so the
    /// modulator only has to be evaluated at note-on.
    pub(crate) fn is_static(&self) -> bool {
        self.source.is_static() && self.amount_source.is_static()
    }

    /// The modulator's current contribution, in the destination generator's
    /// own units.
    pub(crate) fn evaluate(&self, channel: &Channel, key: i32, velocity: i32) -> f32 {
        self.amount as f32
            * self.source.transform(channel, key, velocity)
            * self.amount_source.transform(channel, key, velocity)
    }

    /// The controller driving this modulator.
    pub fn get_source(&self) -> &ModulatorSource {
        &self.source
    }

    /// The generator this modulator adjusts.
    pub fn get_destination(&self) -> u16 {
        self.destination
    }

    /// The modulator's contribution at full scale, in the destination
    /// generator's units.
    pub fn get_amount(&self) -> i16 {
        self.amount
    }

    /// The secondary controller, which scales the modulator's output.
    pub fn get_amount_source(&self) -> &ModulatorSource {
        &self.amount_source
    }

    /// The transform applied to the modulator's output. Only 0, the identity,
    /// is defined by SF2 2.04.
    pub fn get_transform(&self) -> u16 {
        self.transform
    }
}
