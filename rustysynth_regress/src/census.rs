//! Reports what modulators a SoundFont actually carries.
//!
//! Used to check the parser against fonts far too large to commit, and to see
//! which destinations a font leans on before deciding what the engine has to
//! get right.

use std::collections::HashMap;
use std::path::Path;

use crate::open_sound_font;

fn generator_name(destination: u16) -> &'static str {
    match destination {
        5 => "modLfoToPitch",
        6 => "vibLfoToPitch",
        7 => "modEnvToPitch",
        8 => "initialFilterFc",
        9 => "initialFilterQ",
        10 => "modLfoToFilterFc",
        11 => "modEnvToFilterFc",
        13 => "modLfoToVolume",
        15 => "chorusSend",
        16 => "reverbSend",
        17 => "pan",
        21 => "delayModLfo",
        22 => "freqModLfo",
        23 => "delayVibLfo",
        24 => "freqVibLfo",
        25 => "delayModEnv",
        26 => "attackModEnv",
        27 => "holdModEnv",
        28 => "decayModEnv",
        29 => "sustainModEnv",
        30 => "releaseModEnv",
        33 => "delayVolEnv",
        34 => "attackVolEnv",
        35 => "holdVolEnv",
        36 => "decayVolEnv",
        37 => "sustainVolEnv",
        38 => "releaseVolEnv",
        48 => "initialAttenuation",
        51 => "coarseTune",
        52 => "fineTune",
        56 => "scaleTuning",
        _ => "other",
    }
}

fn source_name(index: u8, is_cc: bool) -> String {
    if is_cc {
        return format!("cc{index}");
    }

    match index {
        0 => "none".to_string(),
        2 => "velocity".to_string(),
        3 => "key".to_string(),
        10 => "polyPressure".to_string(),
        13 => "channelPressure".to_string(),
        14 => "pitchWheel".to_string(),
        16 => "pitchWheelSens".to_string(),
        127 => "link".to_string(),
        other => format!("general{other}"),
    }
}

#[derive(Default)]
struct Tally {
    count: usize,
    amounts: Vec<i16>,
}

pub fn run(paths: &[String]) -> Result<(), String> {
    for path in paths {
        let path = Path::new(path);
        let sound_font = open_sound_font(path)?;

        let mut preset_count = 0_usize;
        let mut instrument_count = 0_usize;
        let mut by_destination: HashMap<u16, Tally> = HashMap::new();
        let mut by_source: HashMap<String, usize> = HashMap::new();

        for preset in sound_font.get_presets() {
            for region in preset.get_regions() {
                preset_count += region.get_modulators().len();
            }
        }

        for instrument in sound_font.get_instruments() {
            for region in instrument.get_regions() {
                for modulator in region.get_modulators() {
                    instrument_count += 1;

                    let tally = by_destination
                        .entry(modulator.get_destination())
                        .or_default();
                    tally.count += 1;
                    tally.amounts.push(modulator.get_amount());

                    let source = modulator.get_source();
                    *by_source
                        .entry(source_name(source.get_index(), source.is_midi_controller()))
                        .or_default() += 1;
                }
            }
        }

        println!("{}", path.display());
        println!(
            "  kept {} preset + {} instrument modulators",
            preset_count, instrument_count
        );

        let mut destinations: Vec<(&u16, &Tally)> = by_destination.iter().collect();
        destinations.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        for (destination, tally) in destinations {
            let min = tally.amounts.iter().min().copied().unwrap_or(0);
            let max = tally.amounts.iter().max().copied().unwrap_or(0);
            println!(
                "    dest {:>3} {:<22} {:>5}  amount {}..{}",
                destination,
                generator_name(*destination),
                tally.count,
                min,
                max
            );
        }

        let mut sources: Vec<(&String, &usize)> = by_source.iter().collect();
        sources.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let listed: Vec<String> = sources
            .iter()
            .map(|(name, count)| format!("{name} x{count}"))
            .collect();
        println!("    sources: {}", listed.join(", "));
        println!();
    }

    Ok(())
}
