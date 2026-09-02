//! Measures single notes, to show what honoring modulators actually changed.
//!
//! The corpus comparison answers "did anything change?"; this answers "did the
//! right thing change?". GeneralUser GS drives filter cutoff from velocity in
//! 906 modulators, all of which used to be discarded, so a soft note should now
//! be measurably duller than a loud one on the same patch rather than merely
//! quieter.

use std::path::Path;

use rustysynth::Synthesizer;
use rustysynth::SynthesizerSettings;

use crate::open_sound_font;
use crate::SAMPLE_RATE;

const BLOCK: usize = 64;

struct Measurement {
    peak: f32,
    rms: f32,
    /// A brightness proxy: the mean absolute sample-to-sample difference
    /// divided by the mean absolute level.
    ///
    /// Differencing is a high-pass, so this rises with the share of the energy
    /// sitting at high frequencies and is independent of how loud the note is -
    /// which is what separates "darker" from "quieter".
    brightness: f32,
    /// Level after the note is released, which is essentially the effect tail.
    tail_rms: f32,
}

fn measure(
    synthesizer: &mut Synthesizer,
    key: i32,
    velocity: i32,
    hold_blocks: usize,
    tail_blocks: usize,
) -> Measurement {
    let mut left = vec![0_f32; BLOCK];
    let mut right = vec![0_f32; BLOCK];

    let mut peak = 0_f32;
    let mut sum_squares = 0_f64;
    let mut sum_abs = 0_f64;
    let mut sum_diff = 0_f64;
    let mut count = 0_usize;
    let mut previous = 0_f32;

    let mut tail_squares = 0_f64;
    let mut tail_count = 0_usize;

    synthesizer.note_on(0, key, velocity);

    for block in 0..(hold_blocks + tail_blocks) {
        if block == hold_blocks {
            synthesizer.note_off(0, key);
        }

        synthesizer.render(&mut left[..], &mut right[..]);

        for i in 0..BLOCK {
            let value = 0.5_f32 * (left[i] + right[i]);

            if block < hold_blocks {
                peak = peak.max(value.abs());
                sum_squares += (value as f64) * (value as f64);
                sum_abs += value.abs() as f64;
                sum_diff += (value - previous).abs() as f64;
                count += 1;
            } else {
                tail_squares += (value as f64) * (value as f64);
                tail_count += 1;
            }

            previous = value;
        }
    }

    Measurement {
        peak,
        rms: (sum_squares / count.max(1) as f64).sqrt() as f32,
        brightness: if sum_abs > 0_f64 {
            (sum_diff / sum_abs) as f32
        } else {
            0_f32
        },
        tail_rms: (tail_squares / tail_count.max(1) as f64).sqrt() as f32,
    }
}

fn synthesizer_for(path: &Path, patch: i32, reverb_scale: f32) -> Result<Synthesizer, String> {
    let sound_font = open_sound_font(path)?;

    let mut settings = SynthesizerSettings::new(SAMPLE_RATE);
    settings.reverb_send_scale = reverb_scale;
    settings.chorus_send_scale = reverb_scale;

    let mut synthesizer = Synthesizer::new(&sound_font, &settings).map_err(|e| e.to_string())?;
    synthesizer.process_midi_message(0, 0xC0, patch, 0);
    // Ask for a full reverb send, so the tail measurement reflects what the
    // font allows rather than the default of 40.
    synthesizer.process_midi_message(0, 0xB0, 0x5B, 127);

    Ok(synthesizer)
}

pub fn run(path: &Path, patch: &str) -> Result<(), String> {
    let patch: i32 = patch.parse().map_err(|_| format!("bad patch: {patch}"))?;

    let hold = SAMPLE_RATE as usize / BLOCK; // one second
    let tail = SAMPLE_RATE as usize / BLOCK; // one more

    println!("{} patch {patch}", path.display());
    println!("  velocity response, key 60");
    println!(
        "    {:>4}  {:>10}  {:>10}  {:>10}",
        "vel", "peak", "rms", "bright"
    );

    let mut first: Option<f32> = None;
    let mut last = 0_f32;

    for velocity in [16, 32, 64, 96, 127] {
        let mut synthesizer = synthesizer_for(path, patch, 1_f32)?;
        let m = measure(&mut synthesizer, 60, velocity, hold, 0);

        println!(
            "    {:>4}  {:>10.6}  {:>10.6}  {:>10.6}",
            velocity, m.peak, m.rms, m.brightness
        );

        if first.is_none() {
            first = Some(m.brightness);
        }
        last = m.brightness;
    }

    let quiet = first.unwrap_or(0_f32);
    let ratio = if quiet > 0_f32 { last / quiet } else { 0_f32 };
    println!(
        "  brightness at velocity 127 is {:.3}x that at velocity 16",
        ratio
    );
    if ratio > 1.02_f32 {
        println!("  -> loud notes are brighter, so velocity reaches filter cutoff");
    } else {
        println!("  -> velocity does not reach filter cutoff in this font");
    }

    println!("  reverb send scale, key 60 velocity 100");
    for scale in [1_f32, 2_f32, 3_f32] {
        let mut synthesizer = synthesizer_for(path, patch, scale)?;
        let m = measure(&mut synthesizer, 60, 100, hold, tail);
        println!("    scale {:>4.1}  tail rms {:>10.6}", scale, m.tail_rms);
    }

    println!();
    Ok(())
}
