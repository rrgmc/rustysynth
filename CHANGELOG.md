# v1.4.0

- **SF2 modulators are now implemented.** The `pmod` and `imod` chunks were previously read and
  discarded, so only generators drove synthesis. Fonts that rely on modulators now sound as their
  authors intended - GeneralUser GS alone ships 2,257 of them, and its entire velocity-dependent
  brightness was missing.
- Preset and instrument regions expose their modulators through `get_modulators()`, and `Modulator`
  and `ModulatorSource` are public.
- Channel pressure (aftertouch) and polyphonic key pressure are now received. Channel pressure
  drives vibrato depth through SF2 default modulator 3; it previously had no effect at all.
- Added `SynthesizerSettings::reverb_send_scale` and `chorus_send_scale`, both defaulting to 1.0. A
  font that ships its own CC91 or CC93 modulators overrides the default one outright, and some cap
  the send well below full scale - GeneralUser GS stops at 35% reverb and 30% chorus. These let a
  drier mix than intended be brought back up without editing the font.

Deliberate deviations from SF2 2.04, all of them chosen so that existing output is preserved:

- **Default modulator 2** (velocity to filter cutoff, -2400 cents) is **not** implemented, matching
  FluidSynth, which hard-codes it away. It would darken soft notes on every SoundFont, including
  fonts voiced without it.
- **Default modulator 10** (pitch wheel to fine tune) is handled natively rather than through the
  modulator engine. The spec's formulation gives 198.4 cents at a pitch bend range of 2 semitones
  instead of 200, and the oscillator latches its tuning at note-on, so a modulator on `fineTune`
  would not bend at all. As a consequence, generators 51 and 52 take their modulated value at
  note-on and are not revisited.
- **The reverb and chorus send defaults use amount 1000**, not the spec's 200, so that CC91 and CC93
  keep the 0..100% range this crate has always given them.
- **Linked modulators, non-identity transforms, and modulators targeting sample addressing, ranges
  or overrides are dropped at load time**, per section 8.2.1, rather than being honored partially.

Two audible changes, both measured over a 4,973 file MIDI corpus rendered through TimGM6mb:

- `modLfoToVolume` was tested with `> 0.05` rather than `.abs() > 0.05`, so a **negative**
  modulation-LFO-to-volume was silently ignored. It is now honored. 51 of 299 sampled files changed,
  the largest by 0.85% peak.
- Channel pressure now reaches vibrato depth, as above. 5 further files changed, one by 12%.

Everything else reproduces the previous output. Rendering the corpus sample through fonts whose
modulator chunks had been emptied - leaving only the SF2 defaults - produced no file differing
beyond f32 rounding, which is what establishes that the default modulator table is calibrated to the
hardcoded controller handling it replaced.

Fixed along the way:

- `BiQuadFilter::set_low_pass_filter` divides by `1 + 6 * (resonance - 1)`, which is zero at about
  -1.58 dB. Generators alone cannot reach it, but modulators can, and FluidR3_GM ships forty at
  amount -470. The NaN would have propagated into the reverb and chorus comb filters, which are IIR
  with persistent state, and the output would never have recovered. Resonance is now clamped to the
  SF2 generator range.

# v1.3.6

- Various code clean-ups ([thanks to @sevonj](https://github.com/sinshu/rustysynth/issues/42)).
- Fixed segfault caused by empty `smpl` sub-chunks ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/48)).
- Added sanity check for zero-length loops ([thanks to @eswartz](https://github.com/sinshu/rustysynth/pull/51)).

# v1.3.5

- Improved error reporting for invalid SoundFonts ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/32)).
- Fixed issue where the loop mode was not handled correctly ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/36)).
- All public types now implement the `Debug` trait.

# v1.3.4

- Some minor optimization.
- Fixed an issue where reading certain invalid SoundFonts results in a panic instead of an error ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/31)).

# v1.3.3

- Some minor optimizations.
- Fixed an issue where the pitch bend range was incorrect in certain MIDI files ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/29)).

# v1.3.2

- Added sanity check in loading SoundFonts ([thanks to @sevonj](https://github.com/sinshu/rustysynth/pull/24)).

# v1.3.1

- Now all the error types don't use heap allocation.

# v1.3.0

- Fixed issue where loading large SoundFont files would fail ([thanks to @paxbun](https://github.com/sinshu/rustysynth/pull/12)).
- Error types no longer allocate `String` ([thanks to @paxbun](https://github.com/sinshu/rustysynth/pull/12)).

# v1.2.1

- Minor tweaks to make the code idiomatic.
- Added `get_sample_id` method to `InstrumentRegion` ([thanks to @pomscyth](https://github.com/sinshu/rustysynth/pull/11)).
- Added `get_instrument_id` method to `PresetRegion`.

# v1.2.0

- Added ability to set the loop point when playing MIDI files.
- Added ability to change the playback speed on the fly when playing MIDI files.
- Added doc comments.

# v1.1.2

- Optimized chorus for better performance.

# v1.1.1

- Fixed issue where reading MIDI files with events inserted after EOT would fail ([thanks to @ArthurCose](https://github.com/sinshu/rustysynth/pull/9)).

# v1.1.0

- Error types are now `non_exhaustive`.
- Loading SoundFont3 explicitly fails with an error `SoundFontError::UnsupportedSampleFormat`.

# v1.0.0

- Introduced custom error types for error reporting.
- Removed unnecessary code.

# v0.9.2

- Refactored the entire code to be more idiomatic ([thanks to @joseluis](https://github.com/sinshu/rustysynth/pull/6)).
- Fixed issue where locks occurred during the rendering process.

# v0.9.1

- Modified the API to accommodate multi-threaded applications ([thanks to @sapir](https://github.com/sinshu/rustysynth/pull/5)).

# v0.9.0

- Implemented reverb and chorus.

# v0.1.0

- First release.
