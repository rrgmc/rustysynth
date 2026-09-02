//! Renders a corpus of MIDI files and reduces each to one line, so that two
//! builds of the library can be compared over hundreds of thousands of files
//! without keeping any audio.

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use rustysynth::MidiFile;
use rustysynth::MidiFileSequencer;
use rustysynth::SoundFont;
use rustysynth::Synthesizer;
use rustysynth::SynthesizerSettings;

use crate::open_sound_font;
use crate::SAMPLE_RATE;

/// How much of each file to render. Long enough to get past the introduction
/// of most files, short enough to run a five thousand file sample in minutes.
const SECONDS: usize = 30;

/// Files larger than this are skipped: the corpus contains a few pathological
/// ones, including an 878 MB outlier.
const MAX_BYTES: u64 = 10 * 1024 * 1024;

const BLOCK: usize = 64;

fn is_midi(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("mid") | Some("kar") | Some("midi")
    )
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else if is_midi(&path) {
            if let Ok(metadata) = entry.metadata() {
                if metadata.len() <= MAX_BYTES {
                    found.push(path);
                }
            }
        }
    }
}

/// Writes a deterministic, evenly spread subset of a corpus.
///
/// Evenly spread rather than randomly drawn: corpora of this kind are laid out
/// by collection, so taking every Nth path in sorted order covers far more
/// distinct sources than a random draw of the same size would.
pub fn sample(dir: &Path, count: &str, output: &Path) -> Result<(), String> {
    let count: usize = count.parse().map_err(|_| format!("bad count: {count}"))?;

    let mut found: Vec<PathBuf> = Vec::new();
    walk(dir, &mut found);
    found.sort();

    if found.is_empty() {
        return Err(format!("no MIDI files under {}", dir.display()));
    }

    let stride = found.len().div_ceil(count).max(1);
    let selected: Vec<&PathBuf> = found.iter().step_by(stride).take(count).collect();

    let file = File::create(output).map_err(|e| format!("{}: {e}", output.display()))?;
    let mut writer = BufWriter::new(file);
    for path in &selected {
        writeln!(writer, "{}", path.display()).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "{} files under {}, sampled {} with stride {} into {}",
        found.len(),
        dir.display(),
        selected.len(),
        stride,
        output.display()
    );

    Ok(())
}

struct Digest {
    hash: u64,
    peak: f32,
    non_finite: usize,
}

fn render_one(sound_font: &Arc<SoundFont>, path: &Path) -> Result<Digest, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let midi_file = Arc::new(MidiFile::new(&mut file).map_err(|e| e.to_string())?);

    let settings = SynthesizerSettings::new(SAMPLE_RATE);
    let synthesizer = Synthesizer::new(sound_font, &settings).map_err(|e| e.to_string())?;
    let mut sequencer = MidiFileSequencer::new(synthesizer);
    sequencer.play(&midi_file, false);

    let mut left = vec![0_f32; BLOCK];
    let mut right = vec![0_f32; BLOCK];

    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut peak = 0_f32;
    let mut non_finite = 0_usize;

    let blocks = SECONDS * SAMPLE_RATE as usize / BLOCK;
    for _ in 0..blocks {
        sequencer.render(&mut left[..], &mut right[..]);

        for i in 0..BLOCK {
            for value in [left[i], right[i]] {
                for byte in value.to_bits().to_le_bytes() {
                    hash ^= byte as u64;
                    hash = hash.wrapping_mul(0x0100_0000_01b3);
                }

                if value.is_finite() {
                    peak = peak.max(value.abs());
                } else {
                    non_finite += 1;
                }
            }
        }
    }

    Ok(Digest {
        hash,
        peak,
        non_finite,
    })
}

pub fn run(sound_font_path: &Path, list: &Path, output: &Path) -> Result<(), String> {
    let sound_font = open_sound_font(sound_font_path)?;
    let listing = fs::read_to_string(list).map_err(|e| format!("{}: {e}", list.display()))?;

    let file = File::create(output).map_err(|e| format!("{}: {e}", output.display()))?;
    let mut writer = BufWriter::new(file);

    let mut rendered = 0_usize;
    let mut failed = 0_usize;
    let mut poisoned = 0_usize;

    for path in listing.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match render_one(&sound_font, Path::new(path)) {
            Ok(digest) => {
                rendered += 1;
                if digest.non_finite > 0 {
                    poisoned += 1;
                    eprintln!("NOT FINITE: {path} ({} samples)", digest.non_finite);
                }
                writeln!(
                    writer,
                    "{path}\t{:016x}\t{:.6}\t{}",
                    digest.hash, digest.peak, digest.non_finite
                )
                .map_err(|e| e.to_string())?;
            }
            Err(error) => {
                failed += 1;
                writeln!(writer, "{path}\tFAILED\t0\t0\t{error}").map_err(|e| e.to_string())?;
            }
        }

        if (rendered + failed).is_multiple_of(250) {
            eprintln!("  {} rendered, {} failed", rendered, failed);
        }
    }

    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "{}: {} rendered, {} failed to parse or render, {} produced NaN or infinity",
        sound_font_path.display(),
        rendered,
        failed,
        poisoned
    );

    if poisoned > 0 {
        return Err(format!("{poisoned} files produced non-finite output"));
    }

    Ok(())
}

fn load_rows(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;

    let mut rows: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('\t') {
            rows.insert(key.to_string(), value.to_string());
        }
    }

    Ok(rows)
}

pub fn compare(a: &Path, b: &Path) -> Result<(), String> {
    let left = load_rows(a)?;
    let right = load_rows(b)?;

    let mut differing = 0_usize;
    let mut only_left = 0_usize;
    let mut shown = 0_usize;

    for (path, value) in &left {
        match right.get(path) {
            Some(other) if other == value => {}
            Some(other) => {
                differing += 1;
                if shown < 20 {
                    shown += 1;
                    println!("differs: {path}\n  a: {value}\n  b: {other}");
                }
            }
            None => only_left += 1,
        }
    }

    let only_right = right.keys().filter(|k| !left.contains_key(*k)).count();

    println!(
        "{} rows compared: {} differ, {} only in {}, {} only in {}",
        left.len(),
        differing,
        only_left,
        a.display(),
        only_right,
        b.display()
    );

    Ok(())
}
