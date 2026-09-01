#![allow(dead_code)]
#![allow(unused_imports)]

use crate::golden_render_util::legacy_script;
use crate::golden_render_util::open_timgm6mb;
use crate::golden_render_util::render_script;
use std::fs;
use std::path::PathBuf;

/// Number of 64-sample blocks rendered by the golden script: ~2.2 seconds of
/// tail past the last event, so release envelopes and the reverb/chorus decay
/// are part of what is compared.
const STEPS: usize = 3000;

/// Largest absolute deviation allowed from the reference waveform, against a
/// signal whose peak is ~0.33.
///
/// This is a *tolerance* check, not a bit-exact one, and deliberately so.
/// Implementing the SF2 default modulators replaces `(volume * expression)^2`
/// with `10^(0.05 * -0.1 * (960 * concave(volume) + 960 * concave(expression)))`
/// evaluated through an interpolated table. Those are the same curve, but they
/// are not the same sequence of f32 roundings, so a correct implementation
/// still moves the last bits. The same is true across optimization levels:
/// LLVM reorders the float reductions in the mixing and effect loops, and
/// float addition is not associative.
///
/// The threshold is nonetheless tight enough to catch any real change. The
/// bugs this guards against - the velocity curve losing its 40% attenuation
/// scaling, the send range collapsing by 5x, a default modulator amount being
/// wrong - all move the waveform by percent, not by parts per million.
const MAX_ABS_DIFF: f32 = 2.0e-4;

/// Same idea, applied to the whole run rather than its worst single sample, so
/// a systematic bias too small to trip `MAX_ABS_DIFF` anywhere still fails.
const MAX_RMS_DIFF: f32 = 2.0e-5;

fn reference_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("samples");
    path.push("golden_legacy_timgm6mb.f32");
    path
}

fn load_reference() -> Vec<f32> {
    let bytes = fs::read(reference_path()).unwrap();
    assert!(
        bytes.len().is_multiple_of(4),
        "reference waveform is truncated"
    );

    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

/// Renders the legacy control script and compares it with the reference
/// waveform captured on commit d11c6fb, before SF2 modulators existed.
///
/// Set `RUSTYSYNTH_REGEN_GOLDEN=1` to rewrite the reference. Only do that when
/// a change to the audio path is intended and understood - the whole point of
/// the fixture is that it predates this work.
#[test]
fn legacy_render_is_unchanged() {
    let sound_font = open_timgm6mb();
    let result = render_script(&sound_font, &legacy_script(), STEPS);

    assert_eq!(
        result.non_finite, 0,
        "NaN or infinity reached the output buffer: {result:?}"
    );
    assert!(
        result.nonzero_blocks > STEPS / 2,
        "the script rendered mostly silence, so it proves nothing: {result:?}"
    );

    if std::env::var("RUSTYSYNTH_REGEN_GOLDEN").is_ok() {
        let mut bytes: Vec<u8> = Vec::with_capacity(4 * result.samples.len());
        for value in &result.samples {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(reference_path(), &bytes).unwrap();
        println!(
            "regenerated {} ({} samples, {} bytes)",
            reference_path().display(),
            result.samples.len(),
            bytes.len()
        );
        return;
    }

    let reference = load_reference();
    assert_eq!(
        reference.len(),
        result.samples.len(),
        "reference waveform has a different length than the render; \
         the script or the decimation changed"
    );

    let mut max_abs_diff = 0_f32;
    let mut worst_at = 0_usize;
    let mut sum_squares = 0_f64;

    for (i, (rendered, expected)) in result.samples.iter().zip(reference.iter()).enumerate() {
        let diff = (rendered - expected).abs();
        if diff > max_abs_diff {
            max_abs_diff = diff;
            worst_at = i;
        }
        sum_squares += (diff as f64) * (diff as f64);
    }

    let rms_diff = (sum_squares / reference.len() as f64).sqrt() as f32;

    println!(
        "golden: max abs diff {max_abs_diff:e}, rms diff {rms_diff:e}, peak {}, rms {}",
        result.peak, result.rms
    );

    assert!(
        max_abs_diff <= MAX_ABS_DIFF && rms_diff <= MAX_RMS_DIFF,
        "render drifted from the reference waveform: \
         max abs diff {max_abs_diff:e} (limit {MAX_ABS_DIFF:e}) at sample {worst_at}, \
         rms diff {rms_diff:e} (limit {MAX_RMS_DIFF:e}); peak {}, rms {}",
        result.peak,
        result.rms
    );
}
