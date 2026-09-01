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
        // Sliced rather than indexed, the same way the modulators below are.
        // The generator loop used to index straight into the slice, so a bag
        // record pointing past the end of `pgen` or `igen` panicked instead of
        // reporting an invalid file.
        if info.generator_index < 0 || info.generator_count < 0 {
            return Err(SoundFontError::InvalidGeneratorList);
        }
        let start = info.generator_index as usize;
        let end = start + info.generator_count as usize;
        let segment = generators
            .get(start..end)
            .ok_or(SoundFontError::InvalidGeneratorList)?
            .to_vec();

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
