use crate::error::SoundFontError;
use crate::generator::Generator;
use crate::modulator::Modulator;
use crate::zone_info::ZoneInfo;

#[non_exhaustive]
pub(crate) struct Zone {
    pub(crate) generators: Vec<Generator>,
    pub(crate) modulators: Vec<Modulator>,
}

impl Zone {
    pub(crate) fn empty() -> Self {
        Self {
            generators: Vec::new(),
            modulators: Vec::new(),
        }
    }

    fn new(
        info: &ZoneInfo,
        generators: &[Generator],
        modulators: &[Modulator],
    ) -> Result<Self, SoundFontError> {
        let mut segment: Vec<Generator> = Vec::new();

        for i in 0..info.generator_count {
            segment.push(generators[(info.generator_index + i) as usize]);
        }

        // Sliced rather than indexed. The generator loop above trusts the file,
        // which this crate has been bitten by before; there is no reason to add
        // a second place that does.
        if info.modulator_index < 0 || info.modulator_count < 0 {
            return Err(SoundFontError::InvalidModulatorList);
        }
        let start = info.modulator_index as usize;
        let end = start + info.modulator_count as usize;
        let segment_modulators = modulators
            .get(start..end)
            .ok_or(SoundFontError::InvalidModulatorList)?
            .to_vec();

        Ok(Self {
            generators: segment,
            modulators: segment_modulators,
        })
    }

    pub(crate) fn create(
        infos: &[ZoneInfo],
        generators: &[Generator],
        modulators: &[Modulator],
    ) -> Result<Vec<Zone>, SoundFontError> {
        if infos.len() <= 1 {
            return Err(SoundFontError::ZoneNotFound);
        }

        // The last one is the terminator.
        let count = infos.len() - 1;

        let mut zones: Vec<Zone> = Vec::new();
        for info in infos.iter().take(count) {
            zones.push(Zone::new(info, generators, modulators)?);
        }

        Ok(zones)
    }
}
