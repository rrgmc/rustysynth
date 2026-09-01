#![allow(dead_code)]

use crate::error::SoundFontError;
use crate::instrument_info::InstrumentInfo;
use crate::instrument_region::InstrumentRegion;
use crate::sample_header::SampleHeader;
use crate::soundfont_warning::{SoundFontWarning, WarningCollector};
use crate::zone::Zone;

/// Represents an instrument in the SoundFont.
#[derive(Debug)]
#[non_exhaustive]
pub struct Instrument {
    pub(crate) name: String,
    pub(crate) regions: Vec<InstrumentRegion>,
}

impl Instrument {
    fn new(
        info: &InstrumentInfo,
        instrument_id: usize,
        zones: &[Zone],
        samples: &[SampleHeader],
        warnings: &mut WarningCollector,
    ) -> Result<Self, SoundFontError> {
        let name = info.name.clone();

        // An empty bag span used to reject the whole file. It is what
        // Timbres of Heaven (XGM) 4.00 does at instrument 9, and it costs the
        // other 365 instruments in the bank for nothing: an instrument with no
        // zones simply has no regions.
        //
        // The instrument is kept rather than dropped. Preset regions address
        // instruments by position, so removing one would silently repoint
        // every later preset at the wrong instrument.
        let zone_count = info.zone_end_index - info.zone_start_index + 1;
        if zone_count < 0 || info.zone_start_index < 0 {
            return Err(SoundFontError::InvalidInstrument(instrument_id));
        }
        if zone_count == 0 {
            warnings.push(SoundFontWarning::InstrumentWithoutZone(instrument_id));
            return Ok(Self {
                name,
                regions: Vec::new(),
            });
        }

        let span_start = info.zone_start_index as usize;
        let span_end = span_start + zone_count as usize;
        let zone_span = zones
            .get(span_start..span_end)
            .ok_or(SoundFontError::InvalidZoneList)?;
        let regions = InstrumentRegion::create(instrument_id, zone_span, samples, warnings);

        Ok(Self { name, regions })
    }

    pub(crate) fn create(
        infos: &[InstrumentInfo],
        zones: &[Zone],
        samples: &[SampleHeader],
        warnings: &mut WarningCollector,
    ) -> Result<Vec<Instrument>, SoundFontError> {
        if infos.len() <= 1 {
            return Err(SoundFontError::InstrumentNotFound);
        }

        // The last one is the terminator.
        let count = infos.len() - 1;

        let mut instruments: Vec<Instrument> = Vec::new();
        for (instrument_id, info) in infos.iter().take(count).enumerate() {
            instruments.push(Instrument::new(
                info,
                instrument_id,
                zones,
                samples,
                warnings,
            )?);
        }

        Ok(instruments)
    }

    /// Gets the name of the instrument.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Gets the regions of the instrument.
    pub fn get_regions(&self) -> &[InstrumentRegion] {
        &self.regions[..]
    }
}
