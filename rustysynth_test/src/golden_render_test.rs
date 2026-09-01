#![allow(dead_code)]
#![allow(unused_imports)]

use crate::golden_render_util::legacy_script;
use crate::golden_render_util::open_timgm6mb;
use crate::golden_render_util::render_script;

/// Number of 64-sample blocks rendered by the golden script: ~2.2 seconds of
/// tail past the last event, so release envelopes and the reverb/chorus decay
/// are part of what is hashed.
const STEPS: usize = 3000;

/// Bit-exact hash of the legacy control script rendered through TimGM6mb.sf2,
/// captured on commit d11c6fb - before SF2 modulators were implemented.
///
/// Implementing the default modulator set is supposed to *reproduce* this, not
/// change it: the SF2 defaults for velocity, CC7, CC11, CC10 and CC1 are
/// arithmetically identical to the hardcoded paths they replace. If this hash
/// moves, the default modulator table is wrong - it is not an expected
/// consequence of the feature. See the plan's Verification section.
///
/// The value is profile-dependent because LLVM reorders the float reductions
/// in the mixing and effect loops when optimizing, and float addition is not
/// associative. Each profile is deterministic on its own, and requiring both
/// to reproduce is a stronger check than either alone - so run this test under
/// `cargo test` *and* `cargo test --release`.
#[cfg(debug_assertions)]
const GOLDEN_HASH: u64 = 0x77dd_e795_1019_383a;
#[cfg(not(debug_assertions))]
const GOLDEN_HASH: u64 = 0x90e3_fa56_eb18_f27b;

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

    assert_eq!(
        result.hash, GOLDEN_HASH,
        "golden render changed: {result:?} (hash {:#018x})",
        result.hash
    );
}
