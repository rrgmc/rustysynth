//! Offline verification harness for the SF2 modulator work.
//!
//! Not part of the published library and not run by `cargo test`: it exists to
//! be pointed at SoundFonts and a MIDI corpus that are far too large to commit,
//! and to answer two questions that unit tests cannot.
//!
//! - Does implementing the SF2 default modulators still render what this crate
//!   rendered before? (`render` twice, then `compare`, over a font whose own
//!   modulators have been removed with `strip-mods`.)
//! - Does honoring a font's modulators change anything, and does it ever
//!   produce a NaN? (`render` against the unmodified font.)

use std::env;
use std::fs::File;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use rustysynth::SoundFont;

mod census;
mod load;
mod probe;
mod render;
mod strip;

const SAMPLE_RATE: i32 = 44100;

fn usage() -> ExitCode {
    eprintln!(
        "usage:
  load <sf2>...                         does each font open, and what was dropped
  census <sf2>...                       modulator inventory for each font
  strip-mods <in.sf2> <out.sf2>         copy a font with pmod/imod emptied
  render <sf2> <list.txt> <out.tsv>     render each listed MIDI file, hash it
  sample <dir> <count> <out.txt>        stratified sample of a MIDI corpus
  compare <a.tsv> <b.tsv>               report rows that differ
  probe <sf2> <patch>                   velocity response and send scale of one patch"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first() else {
        return usage();
    };

    let result = match (command.as_str(), args.len()) {
        ("load", n) if n >= 2 => load::run(&args[1..]),
        ("census", n) if n >= 2 => census::run(&args[1..]),
        ("strip-mods", 3) => strip::run(Path::new(&args[1]), Path::new(&args[2])),
        ("render", 4) => render::run(
            Path::new(&args[1]),
            Path::new(&args[2]),
            Path::new(&args[3]),
        ),
        ("sample", 4) => render::sample(Path::new(&args[1]), &args[2], Path::new(&args[3])),
        ("compare", 3) => render::compare(Path::new(&args[1]), Path::new(&args[2])),
        ("probe", 3) => probe::run(Path::new(&args[1]), &args[2]),
        _ => return usage(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

pub fn open_sound_font(path: &Path) -> Result<Arc<SoundFont>, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    SoundFont::new(&mut file)
        .map(Arc::new)
        .map_err(|e| format!("{}: {e}", path.display()))
}
