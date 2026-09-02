//! Localises a rendering complaint to a channel, and to a number.
//!
//! `render` reduces a whole file to one line, which is what makes two builds
//! comparable but also means it cannot answer "which part sounds wrong?". These
//! three commands answer that instead:
//!
//! - `stems` renders each channel on its own, so the ear can pick the offending
//!   part out in one pass rather than by elimination.
//! - `notes` reports what every note-on actually resolved to - preset,
//!   instrument, sample, root key, the three tuning generators - and how far
//!   off equal temperament the result lands. That turns "sounds out of tune"
//!   into cents.
//! - `voices` reports how much polyphony the file really wants, because a
//!   layered patch starts several voices per note and a stolen layer is easy to
//!   mistake for a tuning fault.
//!
//! All three drive `Synthesizer` from `MidiFile::get_events` rather than
//! through `MidiFileSequencer`, which is what lets `stems` drop a channel's
//! events on the floor.

use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use rustysynth::MidiEvent;
use rustysynth::MidiFile;
use rustysynth::SoundFont;
use rustysynth::Synthesizer;
use rustysynth::SynthesizerSettings;

use crate::open_sound_font;
use crate::SAMPLE_RATE;

const BLOCK: usize = 64;

/// Rendered past the last event so release tails are not cut off.
const TAIL_SECONDS: f64 = 2.0;

fn open_midi(path: &Path) -> Result<MidiFile, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    MidiFile::new(&mut file).map_err(|e| format!("{}: {e}", path.display()))
}

/// The events of one channel, or of every channel when `only` is `None`.
fn events_of(midi_file: &MidiFile, only: Option<i32>) -> Vec<MidiEvent> {
    midi_file
        .get_events()
        .filter(|event| only.is_none_or(|channel| event.get_channel() == channel))
        .collect()
}

/// Which channels have a note-on, and how many.
fn note_counts(midi_file: &MidiFile) -> BTreeMap<i32, usize> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();

    for event in midi_file.get_events() {
        if event.get_command() == 0x90 && event.get_data2() != 0 {
            *counts.entry(event.get_channel()).or_insert(0) += 1;
        }
    }

    counts
}

/// Drives the synthesizer over `events` block by block, calling `observe`
/// before each block with the events that block is about to dispatch.
///
/// This mirrors `MidiFileSequencer::render`, including the quantization of
/// events to the block boundary, so what comes out is what the sequencer would
/// have produced for the same event list.
fn play<F>(
    synthesizer: &mut Synthesizer,
    events: &[MidiEvent],
    left: &mut [f32],
    right: &mut [f32],
    mut observe: F,
) where
    F: FnMut(&mut Synthesizer, &[MidiEvent]),
{
    let mut next = 0_usize;
    let mut time = 0_f64;
    let block_seconds = BLOCK as f64 / SAMPLE_RATE as f64;

    for block in 0..(left.len() / BLOCK) {
        let first = next;
        while next < events.len() && events[next].get_time() <= time {
            next += 1;
        }

        observe(synthesizer, &events[first..next]);

        for event in &events[first..next] {
            synthesizer.process_midi_message(
                event.get_channel(),
                event.get_command(),
                event.get_data1(),
                event.get_data2(),
            );
        }

        let from = block * BLOCK;
        let to = from + BLOCK;
        synthesizer.render(&mut left[from..to], &mut right[from..to]);

        time += block_seconds;
    }
}

fn block_count(midi_file: &MidiFile) -> usize {
    let seconds = midi_file.get_length() + TAIL_SECONDS;
    (seconds * SAMPLE_RATE as f64 / BLOCK as f64).ceil() as usize
}

// -------------------------------------------------------------------------
// stems
// -------------------------------------------------------------------------

pub fn stems(sound_font_path: &Path, midi_path: &Path, out_dir: &Path) -> Result<(), String> {
    let sound_font = open_sound_font(sound_font_path)?;
    let midi_file = open_midi(midi_path)?;

    fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;

    let counts = note_counts(&midi_file);
    if counts.is_empty() {
        return Err(format!("{}: no note-on events", midi_path.display()));
    }

    let samples = block_count(&midi_file) * BLOCK;
    println!(
        "{} through {}: {:.1} s, {} channels with notes",
        midi_path.display(),
        sound_font_path.display(),
        midi_file.get_length(),
        counts.len()
    );

    for (&channel, &notes) in &counts {
        let events = events_of(&midi_file, Some(channel));

        let mut left = vec![0_f32; samples];
        let mut right = vec![0_f32; samples];
        let mut synthesizer = new_synthesizer(&sound_font)?;
        play(&mut synthesizer, &events, &mut left, &mut right, |_, _| ());

        let path = out_dir.join(format!("ch{channel:02}.wav"));
        let peak = write_wav(&path, &left, &right)?;

        println!(
            "  ch{channel:02}  {notes:5} notes  peak {peak:.3}  {}",
            path.display()
        );
    }

    let events = events_of(&midi_file, None);
    let mut left = vec![0_f32; samples];
    let mut right = vec![0_f32; samples];
    let mut synthesizer = new_synthesizer(&sound_font)?;
    play(&mut synthesizer, &events, &mut left, &mut right, |_, _| ());

    let path = out_dir.join("mix.wav");
    let peak = write_wav(&path, &left, &right)?;
    println!(
        "  mix     {:5} notes  peak {peak:.3}  {}",
        events.len(),
        path.display()
    );

    Ok(())
}

fn new_synthesizer(sound_font: &Arc<SoundFont>) -> Result<Synthesizer, String> {
    let mut settings = SynthesizerSettings::new(SAMPLE_RATE);
    settings.block_size = BLOCK;
    Synthesizer::new(sound_font, &settings).map_err(|e| e.to_string())
}

/// Writes a 16-bit stereo WAV by hand, and returns the peak that was clipped
/// against. Keeping the harness dependency-free matters less than the library,
/// but a 44-byte header is not worth a crate.
fn write_wav(path: &Path, left: &[f32], right: &[f32]) -> Result<f32, String> {
    let frames = left.len();
    let data_len = (frames * 4) as u32;

    let mut peak = 0_f32;
    for i in 0..frames {
        peak = peak.max(left[i].abs()).max(right[i].abs());
    }

    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut writer = BufWriter::new(file);
    let mut out = |bytes: &[u8]| -> Result<(), String> {
        writer
            .write_all(bytes)
            .map_err(|e| format!("{}: {e}", path.display()))
    };

    out(b"RIFF")?;
    out(&(36 + data_len).to_le_bytes())?;
    out(b"WAVEfmt ")?;
    out(&16_u32.to_le_bytes())?;
    out(&1_u16.to_le_bytes())?; // PCM
    out(&2_u16.to_le_bytes())?; // stereo
    out(&(SAMPLE_RATE as u32).to_le_bytes())?;
    out(&((SAMPLE_RATE * 4) as u32).to_le_bytes())?;
    out(&4_u16.to_le_bytes())?; // block align
    out(&16_u16.to_le_bytes())?;
    out(b"data")?;
    out(&data_len.to_le_bytes())?;

    for i in 0..frames {
        for value in [left[i], right[i]] {
            let clamped = if value.is_finite() {
                value.clamp(-1_f32, 1_f32)
            } else {
                0_f32
            };
            out(&((clamped * 32767_f32) as i16).to_le_bytes())?;
        }
    }

    writer
        .flush()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    Ok(peak)
}

// -------------------------------------------------------------------------
// notes
// -------------------------------------------------------------------------

/// What one note-on resolved to, for one matching region pair.
struct Resolved {
    preset: String,
    instrument: String,
    sample: String,
    root_key: i32,
    scale_tuning: i32,
    coarse_tune: i32,
    fine_tune: i32,
    sample_rate: i32,
    /// Semitones the oscillator shifts the sample by, relative to its root key.
    pitch_change: f64,
    /// How far the sounding pitch lands from the key that was asked for.
    cents_error: f64,
}

/// Mirrors the preset lookup in `Synthesizer::note_on`, including the fallback
/// to the GM sound set, so a trace row says what the synth really chose rather
/// than what the file asked for.
fn find_preset(sound_font: &SoundFont, bank: i32, patch: i32) -> Option<usize> {
    let exact = sound_font
        .get_presets()
        .iter()
        .position(|preset| preset.get_bank_number() == bank && preset.get_patch_number() == patch);
    if exact.is_some() {
        return exact;
    }

    let (fallback_bank, fallback_patch) = if bank < 128 { (0, patch) } else { (128, 0) };
    sound_font.get_presets().iter().position(|preset| {
        preset.get_bank_number() == fallback_bank && preset.get_patch_number() == fallback_patch
    })
}

fn resolve(
    sound_font: &SoundFont,
    bank: i32,
    patch: i32,
    key: i32,
    velocity: i32,
) -> Vec<Resolved> {
    let mut out = Vec::new();

    let Some(preset_index) = find_preset(sound_font, bank, patch) else {
        return out;
    };
    let preset = &sound_font.get_presets()[preset_index];

    for preset_region in preset.get_regions() {
        if !preset_region.contains(key, velocity) {
            continue;
        }

        let instrument = &sound_font.get_instruments()[preset_region.get_instrument_id()];
        for instrument_region in instrument.get_regions() {
            if !instrument_region.contains(key, velocity) {
                continue;
            }

            // The same sums RegionPair::get_* makes: the three tuning
            // generators are preset plus instrument, and the root key comes
            // from the instrument alone. InstrumentRegion::get_fine_tune
            // already folds in the sample's pitch correction.
            let scale_tuning =
                preset_region.get_scale_tuning() + instrument_region.get_scale_tuning();
            let coarse_tune = preset_region.get_coarse_tune() + instrument_region.get_coarse_tune();
            let fine_tune = preset_region.get_fine_tune() + instrument_region.get_fine_tune();
            let root_key = instrument_region.get_root_key();

            let pitch_change = (scale_tuning as f64 / 100_f64) * (key - root_key) as f64
                + coarse_tune as f64
                + 0.01_f64 * fine_tune as f64;

            let sample = &sound_font.get_sample_headers()[instrument_region.get_sample_id()];

            out.push(Resolved {
                preset: preset.get_name().to_string(),
                instrument: instrument.get_name().to_string(),
                sample: sample.get_name().to_string(),
                root_key,
                scale_tuning,
                coarse_tune,
                fine_tune,
                sample_rate: sample.get_sample_rate(),
                pitch_change,
                cents_error: (root_key as f64 + pitch_change - key as f64) * 100_f64,
            });
        }
    }

    out
}

pub fn notes(sound_font_path: &Path, midi_path: &Path, output: &Path) -> Result<(), String> {
    let sound_font = open_sound_font(sound_font_path)?;
    let midi_file = open_midi(midi_path)?;

    let file = File::create(output).map_err(|e| format!("{}: {e}", output.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(
        writer,
        "time\tchannel\tkey\tvelocity\tbank\tpatch\tpreset\tinstrument\tsample\troot_key\tscale_tuning\tcoarse_tune\tfine_tune\tsample_rate\tpitch_change\tcents_error"
    )
    .map_err(|e| e.to_string())?;

    // Bank and program are per channel and change mid-file, so they have to be
    // tracked the way a channel would. Channel 9 is percussion, which the
    // synthesizer forces to bank 128.
    let mut bank = [0_i32; 16];
    let mut patch = [0_i32; 16];
    bank[9] = 128;

    let mut rows = 0_usize;
    let mut silent = 0_usize;
    let mut worst: BTreeMap<i32, (f64, String)> = BTreeMap::new();
    let mut per_channel: BTreeMap<i32, (usize, f64)> = BTreeMap::new();

    for event in midi_file.get_events() {
        let channel = event.get_channel();

        match event.get_command() {
            0xB0 if event.get_data1() == 0x00 => {
                bank[channel as usize] = if channel == 9 {
                    event.get_data2() + 128
                } else {
                    event.get_data2()
                }
            }
            0xC0 => patch[channel as usize] = event.get_data1(),
            0x90 if event.get_data2() != 0 => {
                let key = event.get_data1();
                let velocity = event.get_data2();
                let resolved = resolve(
                    &sound_font,
                    bank[channel as usize],
                    patch[channel as usize],
                    key,
                    velocity,
                );

                if resolved.is_empty() {
                    silent += 1;
                }

                for row in &resolved {
                    writeln!(
                        writer,
                        "{:.4}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{:+.1}",
                        event.get_time(),
                        channel,
                        key,
                        velocity,
                        bank[channel as usize],
                        patch[channel as usize],
                        row.preset,
                        row.instrument,
                        row.sample,
                        row.root_key,
                        row.scale_tuning,
                        row.coarse_tune,
                        row.fine_tune,
                        row.sample_rate,
                        row.pitch_change,
                        row.cents_error,
                    )
                    .map_err(|e| e.to_string())?;
                    rows += 1;

                    let entry = per_channel.entry(channel).or_insert((0, 0_f64));
                    entry.0 += 1;
                    entry.1 += row.cents_error.abs();

                    let seen = worst.entry(channel).or_insert((0_f64, String::new()));
                    if row.cents_error.abs() > seen.0 {
                        *seen = (
                            row.cents_error.abs(),
                            format!(
                                "key {key} -> '{}' / '{}' root {} scale {} ({:+.1} cents)",
                                row.instrument,
                                row.sample,
                                row.root_key,
                                row.scale_tuning,
                                row.cents_error
                            ),
                        );
                    }
                }
            }
            _ => (),
        }
    }

    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "{} through {}: {rows} voice rows written to {}",
        midi_path.display(),
        sound_font_path.display(),
        output.display()
    );
    if silent > 0 {
        println!("  {silent} note-ons matched no region at all - those play silence");
    }

    println!("\n  mean absolute tuning error, and the worst note, per channel:");
    for (channel, (count, total)) in &per_channel {
        let mean = total / *count as f64;
        let detail = worst
            .get(channel)
            .map(|(_, text)| text.as_str())
            .unwrap_or("");
        println!("    ch{channel:02}  {count:5} voices  mean {mean:6.1} cents  worst: {detail}");
    }

    Ok(())
}

// -------------------------------------------------------------------------
// voices
// -------------------------------------------------------------------------

pub fn voices(sound_font_path: &Path, midi_path: &Path) -> Result<(), String> {
    let sound_font = open_sound_font(sound_font_path)?;
    let midi_file = open_midi(midi_path)?;

    let events = events_of(&midi_file, None);
    let samples = block_count(&midi_file) * BLOCK;
    let mut left = vec![0_f32; samples];
    let mut right = vec![0_f32; samples];
    let mut synthesizer = new_synthesizer(&sound_font)?;

    let polyphony = synthesizer.get_maximum_polyphony();

    let mut peak = 0_usize;
    let mut total = 0_usize;
    let mut blocks = 0_usize;
    // A block that dispatched a note-on while already saturated must have
    // stolen a sounding voice to serve it.
    let mut saturated_note_ons = 0_usize;

    play(
        &mut synthesizer,
        &events,
        &mut left,
        &mut right,
        |synthesizer, due| {
            let active = synthesizer.get_active_voice_count();
            peak = peak.max(active);
            total += active;
            blocks += 1;

            if active >= polyphony {
                saturated_note_ons += due
                    .iter()
                    .filter(|event| event.get_command() == 0x90 && event.get_data2() != 0)
                    .count();
            }
        },
    );

    let counts = note_counts(&midi_file);
    let note_ons: usize = counts.values().sum();

    println!(
        "{} through {}: {:.1} s, {note_ons} note-ons on {} channels",
        midi_path.display(),
        sound_font_path.display(),
        midi_file.get_length(),
        counts.len()
    );
    println!("  maximum polyphony   {polyphony}");
    println!("  peak active voices  {peak}");
    println!(
        "  mean active voices  {:.1}",
        total as f64 / blocks.max(1) as f64
    );
    println!("  note-ons that had to steal a voice  {saturated_note_ons}");
    if peak >= polyphony {
        println!(
            "  the pool saturated: raise maximum_polyphony above {polyphony} to hear what this file wants"
        );
    }

    println!("\n  note-ons per channel:");
    for (channel, count) in &counts {
        println!("    ch{channel:02}  {count:5}");
    }

    Ok(())
}
