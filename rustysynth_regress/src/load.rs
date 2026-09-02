//! Reports whether a SoundFont opens, and what it cost to open it.
//!
//! This is the command the leniency work is measured with. The question it
//! answers is the one a karaoke application's survey of fifteen General MIDI
//! banks raised: four of them could not be opened at all, each over a single
//! bad record, and there was no way to see which record without reading the
//! file by hand. Point this at a bank and it says either why it still will not
//! open, or exactly what was dropped to make it.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use std::time::Instant;

use rustysynth::{SoundFont, SoundFontError, SoundFontWarning};

/// Groups warnings by kind so that a font shedding a thousand regions reports
/// one line rather than a thousand.
fn kind(warning: &SoundFontWarning) -> &'static str {
    match warning {
        SoundFontWarning::RegionOutOfRange { .. } => "region out of range",
        SoundFontWarning::RegionInvalidSampleId { .. } => "region with an invalid sample id",
        SoundFontWarning::ZoneWithoutSampleId { .. } => "zone with no sample id",
        SoundFontWarning::PresetZoneWithoutInstrument { .. } => "preset zone with no instrument",
        SoundFontWarning::PresetInvalidInstrumentId { .. } => {
            "preset region with an invalid instrument id"
        }
        SoundFontWarning::InstrumentWithoutZone(_) => "instrument with no zone",
        SoundFontWarning::PresetWithoutZone(_) => "preset with no zone",
        SoundFontWarning::UnknownChunk { .. } => "unknown chunk",
        _ => "other",
    }
}

fn report(sound_font: &SoundFont, millis: u128) {
    let regions: usize = sound_font
        .get_instruments()
        .iter()
        .map(|instrument| instrument.get_regions().len())
        .sum();
    let empty_instruments = sound_font
        .get_instruments()
        .iter()
        .filter(|instrument| instrument.get_regions().is_empty())
        .count();

    println!("  OK in {millis} ms");
    println!(
        "  {} presets, {} instruments ({} with no region), {} instrument regions, {} samples",
        sound_font.get_presets().len(),
        sound_font.get_instruments().len(),
        empty_instruments,
        regions,
        sound_font.get_sample_headers().len()
    );

    if sound_font.get_warning_count() == 0 {
        println!("  no warnings - the font is well formed");
        println!();
        return;
    }

    // The kept list is capped, so the tally below is of what was kept while
    // the total is of what happened. Saying both keeps the difference honest.
    let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
    for warning in sound_font.get_warnings() {
        *by_kind.entry(kind(warning)).or_default() += 1;
    }

    println!(
        "  {} warnings, of which {} were kept:",
        sound_font.get_warning_count(),
        sound_font.get_warnings().len()
    );
    for (kind, count) in &by_kind {
        println!("    {count:>6}  {kind}");
    }

    println!("  the first few, in full:");
    for warning in sound_font.get_warnings().iter().take(8) {
        println!("    {warning}");
    }
    println!();
}

pub fn run(paths: &[String]) -> Result<(), String> {
    for path in paths {
        let path = Path::new(path);
        println!("{}", path.display());

        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) => {
                println!("  cannot be opened: {error}");
                println!();
                continue;
            }
        };

        let started = Instant::now();
        let result: Result<SoundFont, SoundFontError> = SoundFont::new(&mut file);
        let millis = started.elapsed().as_millis();

        match result {
            Ok(sound_font) => report(&sound_font, millis),
            Err(error) => {
                // Not returned: a font that will not open is a result, not a
                // reason to stop looking at the rest of the list.
                println!("  FAIL after {millis} ms: {error}");
                println!();
            }
        }
    }

    Ok(())
}
