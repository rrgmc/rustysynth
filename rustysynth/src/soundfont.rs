#![allow(dead_code)]

use std::io::Read;

use crate::binary_reader::BinaryReader;
use crate::error::SoundFontError;
use crate::four_cc::FourCC;
use crate::instrument::Instrument;
use crate::instrument_region::InstrumentRegion;
use crate::preset::Preset;
use crate::sample_header::SampleHeader;
use crate::soundfont_info::SoundFontInfo;
use crate::soundfont_parameters::SoundFontParameters;
use crate::soundfont_sampledata::SoundFontSampleData;
use crate::soundfont_warning::{RegionDefect, SoundFontWarning, WarningCollector};
use crate::LoopMode;

/// Reperesents a SoundFont.
#[derive(Debug)]
#[non_exhaustive]
pub struct SoundFont {
    pub(crate) info: SoundFontInfo,
    pub(crate) bits_per_sample: i32,
    pub(crate) wave_data: Vec<i16>,
    pub(crate) sample_headers: Vec<SampleHeader>,
    pub(crate) presets: Vec<Preset>,
    pub(crate) instruments: Vec<Instrument>,
    pub(crate) warnings: Vec<SoundFontWarning>,
    pub(crate) warning_count: usize,
}

impl SoundFont {
    /// Loads a SoundFont from the stream.
    ///
    /// # Arguments
    ///
    /// * `reader` - The data stream used to load the SoundFont.
    pub fn new<R: Read>(reader: &mut R) -> Result<Self, SoundFontError> {
        let chunk_id = BinaryReader::read_four_cc(reader)?;
        if chunk_id != b"RIFF" {
            return Err(SoundFontError::RiffChunkNotFound);
        }

        let _size = BinaryReader::read_i32(reader)?;

        let form_type = BinaryReader::read_four_cc(reader)?;
        if form_type != b"sfbk" {
            return Err(SoundFontError::InvalidRiffChunkType {
                expected: FourCC::from_bytes(*b"sfbk"),
                actual: form_type,
            });
        }

        let mut collector = WarningCollector::new();

        let info = SoundFontInfo::new(reader, &mut collector)?;
        let sample_data = SoundFontSampleData::new(reader, &mut collector)?;
        let parameters = SoundFontParameters::new(reader, &mut collector)?;

        let mut sound_font = Self {
            info,
            bits_per_sample: sample_data.bits_per_sample,
            wave_data: sample_data.wave_data,
            sample_headers: parameters.sample_headers,
            presets: parameters.presets,
            instruments: parameters.instruments,
            warnings: Vec::new(),
            warning_count: 0,
        };

        sound_font.drop_unplayable_regions(&mut collector)?;

        let (warnings, warning_count) = collector.into_parts();
        sound_font.warnings = warnings;
        sound_font.warning_count = warning_count;

        Ok(sound_font)
    }

    /// Drops every instrument region whose resolved sample addressing cannot be
    /// played, and records each one.
    ///
    /// The conditions are unchanged from when failing any one of them rejected
    /// the whole file; what changed is the scope. Crisis General Midi 3.01 has
    /// exactly one bad record out of 5,007 sample headers, and refusing its
    /// other 1,611 MiB over that served nobody. The checks themselves still
    /// matter - they are what stops the oscillator indexing outside the wave
    /// data - so a region that fails one is dropped rather than tolerated.
    ///
    /// The issues they came from:
    /// <https://github.com/sinshu/rustysynth/issues/22>,
    /// <https://github.com/sinshu/rustysynth/issues/33>,
    /// <https://github.com/sinshu/rustysynth/pull/51>.
    fn drop_unplayable_regions(
        &mut self,
        warnings: &mut WarningCollector,
    ) -> Result<(), SoundFontError> {
        let wave_data_len = self.wave_data.len();
        let mut kept: usize = 0;

        for (instrument_id, instrument) in self.instruments.iter_mut().enumerate() {
            let mut region_index: usize = 0;
            instrument.regions.retain(|region| {
                let index = region_index;
                region_index += 1;

                match SoundFont::region_defect(region, wave_data_len) {
                    None => {
                        kept += 1;
                        true
                    }
                    Some(defect) => {
                        warnings.push(SoundFontWarning::RegionOutOfRange {
                            instrument_id,
                            region_index: index,
                            defect,
                        });
                        false
                    }
                }
            });
        }

        // A font with nothing left to play is not a font, and saying so is more
        // use than handing back something that renders silence.
        if kept == 0 {
            return Err(SoundFontError::SanityCheckFailed);
        }

        Ok(())
    }

    /// The first condition the region fails, or `None` if it is playable.
    ///
    /// The wave data bounds are `>=` rather than `>`: the oscillator
    /// interpolates between `data[index]` and `data[index + 1]` for every index
    /// below the end, so the end has to be a valid index itself. A negative end
    /// casts to a huge `usize` and is caught by the same comparison.
    fn region_defect(region: &InstrumentRegion, wave_data_len: usize) -> Option<RegionDefect> {
        let start = region.get_sample_start();
        let end = region.get_sample_end();
        let start_loop = region.get_sample_start_loop();
        let end_loop = region.get_sample_end_loop();
        let loop_mode = region.get_sample_modes();

        if start < 0 {
            Some(RegionDefect::NegativeStart)
        } else if start_loop < 0 {
            Some(RegionDefect::NegativeLoopStart)
        } else if end as usize >= wave_data_len {
            Some(RegionDefect::EndPastWaveData)
        } else if end_loop as usize >= wave_data_len {
            Some(RegionDefect::LoopEndPastWaveData)
        } else if end <= start {
            Some(RegionDefect::EmptySample)
        } else if end_loop < start_loop {
            Some(RegionDefect::InvertedLoop)
        } else if loop_mode != LoopMode::NoLoop && start_loop >= end_loop {
            Some(RegionDefect::EmptyLoop)
        } else {
            None
        }
    }

    /// Gets the information of the SoundFont.
    pub fn get_info(&self) -> &SoundFontInfo {
        &self.info
    }

    /// Gets the bits per sample of the sample data.
    pub fn get_bits_per_sample(&self) -> i32 {
        self.bits_per_sample
    }

    /// Gets the sample data.
    pub fn get_wave_data(&self) -> &[i16] {
        &self.wave_data[..]
    }

    /// Gets the samples of the SoundFont.
    pub fn get_sample_headers(&self) -> &[SampleHeader] {
        &self.sample_headers[..]
    }

    /// Gets the presets of the SoundFont.
    pub fn get_presets(&self) -> &[Preset] {
        &self.presets[..]
    }

    /// Gets the instruments of the SoundFont.
    pub fn get_instruments(&self) -> &[Instrument] {
        &self.instruments[..]
    }

    /// Gets what the SoundFont got wrong, and what was done about it.
    ///
    /// Empty for a well-formed font. A non-empty list means the font loaded
    /// with records dropped, which is worth surfacing: it is the difference
    /// between "this bank plays" and "this bank plays as its author intended".
    ///
    /// Only the first few are kept - see [`SoundFont::get_warning_count`] for
    /// how many there were altogether.
    pub fn get_warnings(&self) -> &[SoundFontWarning] {
        &self.warnings[..]
    }

    /// Gets how many warnings the load produced, including any past the number
    /// [`SoundFont::get_warnings`] keeps.
    pub fn get_warning_count(&self) -> usize {
        self.warning_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::default_modulators::DEFAULT_MODULATORS;
    use crate::generator_type::GeneratorType;
    use crate::modulator::Modulator;
    use crate::modulator_source::ModulatorSource;
    use std::{fs::File, path::PathBuf};

    fn samples_dir_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("samples")
    }

    #[test]
    fn test_load_reject_sf3() {
        let path = samples_dir_path().join("dummy.sf3");
        let mut file = File::open(&path).unwrap();
        assert!(matches!(
            SoundFont::new(&mut file),
            Err(SoundFontError::UnsupportedSampleFormat)
        ));
    }

    // smpl sub-chunk exists, but is zero-length.
    #[test]
    fn test_load_empty_samples() {
        let path = samples_dir_path().join("test_empty_samples.sf2");
        let mut file = File::open(&path).unwrap();
        assert!(matches!(
            SoundFont::new(&mut file),
            Err(SoundFontError::SampleDataNotFound)
        ));
    }
    /// Loads a fixture built to carry modulators, so the whole load path -
    /// chunk reading, zone slicing, the merge rule and the rejection rules -
    /// is covered without needing a SoundFont too large to commit.
    #[test]
    fn test_load_modulators() {
        let path = samples_dir_path().join("test_modulators.sf2");
        let mut file = File::open(&path).unwrap();
        let sound_font = SoundFont::new(&mut file).unwrap();

        let instrument_region = &sound_font.get_instruments()[0].get_regions()[0];

        // The fixture carries three instrument modulators. The third names a
        // linked source, which this build does not support, so it is dropped
        // at load time rather than reaching the audio thread.
        let modulators = instrument_region.get_modulators();
        assert_eq!(modulators.len(), 2);

        let velocity = &modulators[0];
        assert_eq!(
            velocity.get_destination(),
            GeneratorType::INITIAL_ATTENUATION
        );
        assert_eq!(velocity.get_amount(), 800);
        assert_eq!(
            velocity.get_source().get_index(),
            ModulatorSource::NOTE_ON_VELOCITY
        );
        assert!(!velocity.get_source().is_midi_controller());
        assert!(velocity.get_source().is_negative_direction());
        assert_eq!(
            velocity.get_source().get_curve_type(),
            ModulatorSource::CURVE_CONCAVE
        );

        let brightness = &modulators[1];
        assert_eq!(
            brightness.get_destination(),
            GeneratorType::INITIAL_FILTER_CUTOFF_FREQUENCY
        );
        assert_eq!(brightness.get_amount(), 2400);
        assert!(brightness.get_source().is_midi_controller());
        assert_eq!(brightness.get_source().get_index(), 74);

        // The resolved list is the defaults with the font merged over them.
        // The velocity modulator is an identity match for default 1, so it
        // replaces it at 800 cB rather than stacking to 1760; the CC74 one has
        // no counterpart and is appended.
        let resolved = &instrument_region.resolved_modulators;
        assert_eq!(resolved.len(), DEFAULT_MODULATORS.len() + 1);

        let attenuation_from_velocity: Vec<&Modulator> = resolved
            .iter()
            .filter(|m| {
                m.get_destination() == GeneratorType::INITIAL_ATTENUATION
                    && !m.get_source().is_midi_controller()
            })
            .collect();
        assert_eq!(attenuation_from_velocity.len(), 1);
        assert_eq!(attenuation_from_velocity[0].get_amount(), 800);

        // Preset modulators stay on the preset region, and get no defaults of
        // their own - those belong to the instrument layer, and merging them
        // here as well would apply every default twice.
        let preset_region = &sound_font.get_presets()[0].get_regions()[0];
        let preset_modulators = preset_region.get_modulators();
        assert_eq!(preset_modulators.len(), 1);
        assert_eq!(
            preset_modulators[0].get_destination(),
            GeneratorType::REVERB_EFFECTS_SEND
        );
        assert_eq!(preset_modulators[0].get_amount(), 350);
    }

    /// Every fixture below is built by `samples/make_test_malformed.py`, one
    /// defect apiece.
    fn load_fixture(name: &str) -> Result<SoundFont, SoundFontError> {
        let path = samples_dir_path().join(name);
        let mut file = File::open(&path).unwrap();
        SoundFont::new(&mut file)
    }

    /// The Crisis General Midi case: one bad sample header out of thousands
    /// used to cost the whole file.
    #[test]
    fn test_load_drops_only_the_unplayable_region() {
        let sound_font = load_fixture("test_bad_region.sf2").unwrap();

        let regions = sound_font.get_instruments()[0].get_regions();
        assert_eq!(regions.len(), 1);
        // The survivor is the one on the good sample, not the one that was
        // simply first.
        assert_eq!(regions[0].get_sample_end_loop(), 92);

        assert_eq!(sound_font.get_warning_count(), 1);
        assert_eq!(
            sound_font.get_warnings(),
            [SoundFontWarning::RegionOutOfRange {
                instrument_id: 0,
                region_index: 1,
                defect: RegionDefect::LoopEndPastWaveData,
            }]
        );
    }

    /// The Timbres of Heaven case. The empty instrument has to keep its
    /// position, because preset regions address instruments by index and
    /// removing it would repoint every later preset at the wrong one.
    #[test]
    fn test_load_keeps_an_instrument_that_has_no_zone() {
        let sound_font = load_fixture("test_empty_instrument.sf2").unwrap();

        let instruments = sound_font.get_instruments();
        assert_eq!(instruments.len(), 3);
        assert_eq!(instruments[0].get_regions().len(), 1);
        assert!(instruments[1].get_regions().is_empty());
        assert_eq!(instruments[2].get_regions().len(), 1);

        // Instrument 2 still holds the sample it was given, so nothing shifted
        // underneath it.
        assert_eq!(instruments[2].get_regions()[0].get_sample_start(), 128);

        let preset_regions = sound_font.get_presets()[0].get_regions();
        assert_eq!(preset_regions.len(), 2);
        assert_eq!(preset_regions[0].get_instrument_id(), 0);
        assert_eq!(preset_regions[1].get_instrument_id(), 2);

        assert_eq!(
            sound_font.get_warnings(),
            [SoundFontWarning::InstrumentWithoutZone(1)]
        );
    }

    /// A zone with no `sampleID` used to bind silently to sample 0.
    #[test]
    fn test_load_drops_a_zone_that_names_no_sample() {
        let sound_font = load_fixture("test_zone_without_sample.sf2").unwrap();

        // Zone 0 is the global zone, zone 1 the only real region, and zone 2
        // the one that names nothing.
        assert_eq!(sound_font.get_instruments()[0].get_regions().len(), 1);
        assert_eq!(
            sound_font.get_warnings(),
            [SoundFontWarning::ZoneWithoutSampleId {
                instrument_id: 0,
                zone_index: 2,
            }]
        );
    }

    /// Leniency stops short of handing back a font that can only render
    /// silence.
    #[test]
    fn test_load_reject_when_no_region_survives() {
        assert!(matches!(
            load_fixture("test_no_usable_region.sf2"),
            Err(SoundFontError::SanityCheckFailed)
        ));
    }

    /// An unrecognized four-CC in any list used to be fatal.
    #[test]
    fn test_load_skips_unknown_chunks() {
        let sound_font = load_fixture("test_unknown_chunk.sf2").unwrap();

        assert_eq!(sound_font.get_instruments()[0].get_regions().len(), 1);
        assert_eq!(sound_font.get_warning_count(), 2);

        let lists: Vec<String> = sound_font
            .get_warnings()
            .iter()
            .map(|warning| match warning {
                SoundFontWarning::UnknownChunk { list, .. } => list.to_string(),
                other => panic!("unexpected warning: {other:?}"),
            })
            .collect();
        assert_eq!(lists, ["INFO", "pdta"]);
    }

    /// RIFF pads an odd-sized chunk, and nothing used to consume that byte, so
    /// everything after it read misaligned.
    #[test]
    fn test_load_odd_sized_chunk() {
        let sound_font = load_fixture("test_odd_chunk.sf2").unwrap();

        assert_eq!(sound_font.get_info().get_comments(), "odd");
        assert_eq!(sound_font.get_instruments()[0].get_regions().len(), 1);
        assert_eq!(sound_font.get_warning_count(), 0);
    }

    /// A well-formed font must not acquire warnings just because they now
    /// exist.
    #[test]
    fn test_load_a_good_font_warns_about_nothing() {
        let path = samples_dir_path().join("test_modulators.sf2");
        let mut file = File::open(&path).unwrap();
        let sound_font = SoundFont::new(&mut file).unwrap();

        assert!(sound_font.get_warnings().is_empty());
        assert_eq!(sound_font.get_warning_count(), 0);
    }
}
