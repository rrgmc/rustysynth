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

    /// A modulator that contributes nothing, used to fill the fixed-size array
    /// a voice keeps its dynamic modulators in.
    pub(crate) const fn inactive() -> Self {
        let none = ModulatorSource::general(
            ModulatorSource::NO_CONTROLLER,
            ModulatorSource::CURVE_LINEAR,
            false,
            false,
        );

        Self {
            source: none,
            destination: GeneratorType::UNUSED_END,
            amount: 0,
            amount_source: none,
            transform: 0,
        }
    }

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

    /// Applies the SF2 2.04 section 9.5.4 merge rule to a modulator list.
    ///
    /// A modulator identical to one already present *replaces* it, taking its
    /// amount with it; anything else is appended. Unlike generators, which the
    /// preset layer adds as offsets, an identical modulator is an override -
    /// which is how GeneralUser GS softens the velocity curve from 960 cB to
    /// 800, and how 65 of its regions disable it outright with amount 0.
    ///
    /// The rule applies within a single zone's list as well as between zones,
    /// so later always wins. Unsupported modulators are dropped here rather
    /// than at note-on.
    pub(crate) fn merge(target: &mut Vec<Modulator>, incoming: &[Modulator]) {
        for modulator in incoming.iter() {
            if !modulator.is_supported() {
                continue;
            }

            match target
                .iter_mut()
                .find(|existing| existing.is_identical(modulator))
            {
                Some(existing) => *existing = *modulator,
                None => target.push(*modulator),
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModulatorSource = ModulatorSource::general(
        ModulatorSource::NO_CONTROLLER,
        ModulatorSource::CURVE_LINEAR,
        false,
        false,
    );

    fn velocity_to_attenuation(amount: i16) -> Modulator {
        Modulator {
            source: ModulatorSource::general(
                ModulatorSource::NOTE_ON_VELOCITY,
                ModulatorSource::CURVE_CONCAVE,
                false,
                true,
            ),
            destination: GeneratorType::INITIAL_ATTENUATION,
            amount,
            amount_source: NONE,
            transform: 0,
        }
    }

    fn cc_to(index: u8, destination: u16, amount: i16) -> Modulator {
        Modulator {
            source: ModulatorSource::cc(index, ModulatorSource::CURVE_LINEAR, false, false),
            destination,
            amount,
            amount_source: NONE,
            transform: 0,
        }
    }

    /// The rule that makes the whole feature work: a font overrides a default
    /// by shipping a modulator with the same source and destination, and the
    /// amount is replaced rather than added to.
    ///
    /// GeneralUser GS relies on this twice over - it softens velocity to
    /// attenuation from 960 cB to 800, and disables it outright with amount 0
    /// in 65 regions. Were these appended instead, the two would stack to
    /// 1760 cB and every one of those regions would be far too quiet.
    #[test]
    fn an_identical_modulator_replaces_rather_than_adds() {
        let mut resolved = vec![velocity_to_attenuation(960)];

        Modulator::merge(&mut resolved, &[velocity_to_attenuation(800)]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].get_amount(), 800);

        // Amount 0 disables the default entirely, and has to be honored as an
        // override rather than skipped as a no-op.
        Modulator::merge(&mut resolved, &[velocity_to_attenuation(0)]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].get_amount(), 0);
    }

    #[test]
    fn a_different_destination_is_appended() {
        let mut resolved = vec![velocity_to_attenuation(960)];

        Modulator::merge(
            &mut resolved,
            &[cc_to(91, GeneratorType::REVERB_EFFECTS_SEND, 350)],
        );

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[1].get_amount(), 350);
    }

    /// The rule applies within one list as well as between zones, so a zone
    /// that names the same modulator twice keeps the later one.
    #[test]
    fn later_wins_within_a_single_list() {
        let mut resolved: Vec<Modulator> = Vec::new();

        Modulator::merge(
            &mut resolved,
            &[velocity_to_attenuation(500), velocity_to_attenuation(700)],
        );

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].get_amount(), 700);
    }

    #[test]
    fn unsupported_modulators_are_dropped_at_merge_time() {
        let mut linked = velocity_to_attenuation(960);
        linked.destination |= 0x8000;

        let mut linked_source = velocity_to_attenuation(960);
        linked_source.source = ModulatorSource::general(
            ModulatorSource::LINK,
            ModulatorSource::CURVE_LINEAR,
            false,
            false,
        );

        // The absolute-value transform breaks the identity that lets preset and
        // instrument amounts be summed at voice start.
        let mut transformed = velocity_to_attenuation(960);
        transformed.transform = 2;

        // Sample addressing decides which audio plays, not how it sounds.
        let sample_address = cc_to(74, GeneratorType::START_ADDRESS_OFFSET, 100);
        let sample_id = cc_to(74, GeneratorType::SAMPLE_ID, 1);
        let exclusive = cc_to(74, GeneratorType::EXCLUSIVE_CLASS, 1);

        // Bank select carries a parameter number, not a continuous value.
        let bank_select = cc_to(0, GeneratorType::INITIAL_ATTENUATION, 100);
        let rpn = cc_to(101, GeneratorType::INITIAL_ATTENUATION, 100);

        let mut out_of_range = velocity_to_attenuation(960);
        out_of_range.destination = GeneratorType::COUNT as u16 + 5;

        let rejected = [
            linked,
            linked_source,
            transformed,
            sample_address,
            sample_id,
            exclusive,
            bank_select,
            rpn,
            out_of_range,
        ];

        for modulator in rejected.iter() {
            assert!(
                !modulator.is_supported(),
                "should have been rejected: {modulator:?}"
            );
        }

        let mut resolved: Vec<Modulator> = Vec::new();
        Modulator::merge(&mut resolved, &rejected);
        assert!(resolved.is_empty());

        // A plain, legal one still gets through.
        assert!(velocity_to_attenuation(960).is_supported());
        assert!(cc_to(91, GeneratorType::REVERB_EFFECTS_SEND, 350).is_supported());
    }

    #[test]
    fn only_velocity_and_key_sourced_modulators_are_static() {
        assert!(velocity_to_attenuation(960).is_static());
        assert!(!cc_to(7, GeneratorType::INITIAL_ATTENUATION, 960).is_static());

        // A static source with a dynamic amount source is dynamic overall.
        let mut mixed = velocity_to_attenuation(960);
        mixed.amount_source = ModulatorSource::cc(7, ModulatorSource::CURVE_LINEAR, false, false);
        assert!(!mixed.is_static());
    }

    #[test]
    fn a_modulator_list_of_only_a_terminator_is_legal_and_empty() {
        let bytes = [0_u8; 10];
        let modulators = Modulator::read_from_chunk(&mut &bytes[..], 10).unwrap();
        assert!(modulators.is_empty());

        // A size that is not a whole number of records is not.
        let bytes = [0_u8; 15];
        assert!(matches!(
            Modulator::read_from_chunk(&mut &bytes[..], 15),
            Err(SoundFontError::InvalidModulatorList)
        ));
        assert!(matches!(
            Modulator::read_from_chunk(&mut &bytes[..], 0),
            Err(SoundFontError::InvalidModulatorList)
        ));
    }
}
