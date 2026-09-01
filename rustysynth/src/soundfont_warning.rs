use std::fmt;

use crate::four_cc::FourCC;

/// Why a single instrument region was rejected as unplayable.
///
/// Each variant names one of the conditions `SoundFont`'s sanity check applies
/// to a region's resolved sample addressing. The conditions themselves are
/// unchanged from when failing any of them rejected the whole file; naming the
/// one that failed is what makes the result actionable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionDefect {
    /// The sample start, after the region's address offsets, is negative.
    NegativeStart,
    /// The loop start, after the region's address offsets, is negative.
    NegativeLoopStart,
    /// The sample end addresses past the end of the wave data.
    ///
    /// The bound is one below the length rather than the length itself: the
    /// oscillator interpolates between `data[index]` and `data[index + 1]`
    /// for every `index` below the end, so the end must be a valid index.
    EndPastWaveData,
    /// The loop end addresses past the end of the wave data.
    LoopEndPastWaveData,
    /// The sample end is at or before its start, so there is nothing to play.
    EmptySample,
    /// The loop end is before the loop start.
    InvertedLoop,
    /// The loop is empty, on a region that asks to loop.
    EmptyLoop,
}

impl fmt::Display for RegionDefect {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RegionDefect::NegativeStart => write!(f, "the sample start is negative"),
            RegionDefect::NegativeLoopStart => write!(f, "the loop start is negative"),
            RegionDefect::EndPastWaveData => {
                write!(f, "the sample end is past the end of the wave data")
            }
            RegionDefect::LoopEndPastWaveData => {
                write!(f, "the loop end is past the end of the wave data")
            }
            RegionDefect::EmptySample => write!(f, "the sample end is at or before its start"),
            RegionDefect::InvertedLoop => write!(f, "the loop end is before the loop start"),
            RegionDefect::EmptyLoop => write!(f, "the loop is empty"),
        }
    }
}

/// Something a SoundFont got wrong that was worked around at load time.
///
/// A SoundFont that trips any of these still loads; the record that caused it
/// is dropped and everything else plays. They exist because rejecting a whole
/// bank over one bad record is worse for the person holding the file than
/// playing the other 5,006 samples in it, and because "sanity check failed"
/// with nothing attached tells them nothing they can act on.
///
/// Like [`crate::SoundFontError`], these allocate nothing: they carry indices
/// and four-CCs, never names or strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SoundFontWarning {
    /// An instrument region's resolved sample addressing is unplayable, so the
    /// region was dropped.
    RegionOutOfRange {
        instrument_id: usize,
        region_index: usize,
        defect: RegionDefect,
    },
    /// An instrument region names a sample that does not exist, so the region
    /// was dropped.
    RegionInvalidSampleId {
        instrument_id: usize,
        sample_id: usize,
    },
    /// A non-global instrument zone carries no `sampleID` generator, so it was
    /// dropped rather than being bound to sample 0. SF2 2.04 section 7.7
    /// requires `sampleID` to terminate every such zone.
    ZoneWithoutSampleId {
        instrument_id: usize,
        zone_index: usize,
    },
    /// A non-global preset zone carries no `instrument` generator, so it was
    /// dropped rather than being bound to instrument 0.
    PresetZoneWithoutInstrument { preset_id: usize, zone_index: usize },
    /// A preset region names an instrument that does not exist, so the region
    /// was dropped.
    PresetInvalidInstrumentId {
        preset_id: usize,
        instrument_id: usize,
    },
    /// An instrument's bag span is empty, so it has no regions. The instrument
    /// itself is kept, because preset regions address instruments by position.
    InstrumentWithoutZone(usize),
    /// A preset has no usable zone - its bag span is empty, or every zone in
    /// it was dropped - so the preset was dropped. Keeping it would be worse:
    /// it would be found by a bank and patch lookup, play nothing, and
    /// suppress the fallback to bank 0.
    PresetWithoutZone(usize),
    /// A list held a chunk this crate does not know, which was skipped.
    UnknownChunk { list: FourCC, id: FourCC },
}

impl fmt::Display for SoundFontWarning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SoundFontWarning::RegionOutOfRange {
                instrument_id,
                region_index,
                defect,
            } => write!(
                f,
                "region {region_index} of the instrument with the ID '{instrument_id}' was dropped: {defect}"
            ),
            SoundFontWarning::RegionInvalidSampleId {
                instrument_id,
                sample_id,
            } => write!(
                f,
                "a region of the instrument with the ID '{instrument_id}' was dropped: it names the invalid sample ID '{sample_id}'"
            ),
            SoundFontWarning::ZoneWithoutSampleId {
                instrument_id,
                zone_index,
            } => write!(
                f,
                "zone {zone_index} of the instrument with the ID '{instrument_id}' was dropped: it has no sample ID"
            ),
            SoundFontWarning::PresetZoneWithoutInstrument {
                preset_id,
                zone_index,
            } => write!(
                f,
                "zone {zone_index} of the preset with the ID '{preset_id}' was dropped: it has no instrument"
            ),
            SoundFontWarning::PresetInvalidInstrumentId {
                preset_id,
                instrument_id,
            } => write!(
                f,
                "a region of the preset with the ID '{preset_id}' was dropped: it names the invalid instrument ID '{instrument_id}'"
            ),
            SoundFontWarning::InstrumentWithoutZone(instrument_id) => write!(
                f,
                "the instrument with the ID '{instrument_id}' has no zone, so it has no regions"
            ),
            SoundFontWarning::PresetWithoutZone(preset_id) => write!(
                f,
                "the preset with the ID '{preset_id}' has no usable zone, so it was dropped"
            ),
            SoundFontWarning::UnknownChunk { list, id } => write!(
                f,
                "the '{list}' list contains an unknown ID '{id}', which was skipped"
            ),
        }
    }
}

/// Collects load warnings, keeping a bounded number of them.
///
/// The cap matters: a font with one bad region produces one warning, but a
/// file that is simply not a SoundFont can produce one per region, and a
/// diagnostic that grows without bound is a worse failure than the one it
/// describes. Everything is counted; only the first few are kept.
#[derive(Debug, Default)]
pub(crate) struct WarningCollector {
    warnings: Vec<SoundFontWarning>,
    count: usize,
}

impl WarningCollector {
    const CAPACITY: usize = 64;

    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, warning: SoundFontWarning) {
        self.count += 1;
        if self.warnings.len() < WarningCollector::CAPACITY {
            self.warnings.push(warning);
        }
    }

    pub(crate) fn into_parts(self) -> (Vec<SoundFontWarning>, usize) {
        (self.warnings, self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collector_counts_past_what_it_keeps() {
        let mut collector = WarningCollector::new();
        for i in 0..(WarningCollector::CAPACITY + 10) {
            collector.push(SoundFontWarning::InstrumentWithoutZone(i));
        }

        let (warnings, count) = collector.into_parts();
        assert_eq!(warnings.len(), WarningCollector::CAPACITY);
        assert_eq!(count, WarningCollector::CAPACITY + 10);
        assert_eq!(warnings[0], SoundFontWarning::InstrumentWithoutZone(0));
    }
}
