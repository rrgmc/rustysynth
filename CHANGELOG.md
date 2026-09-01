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

Three audible changes, all measured over a 4,973 file MIDI corpus rendered through TimGM6mb with
its modulator chunks emptied, so that only the SF2 defaults were in play. 4,155 of the 4,908
renderable files (**84.7%**) render at an unchanged level; the rest divide as follows.

- `modLfoToVolume` was tested with `> 0.05` rather than `.abs() > 0.05`, so a **negative**
  modulation-LFO-to-volume was silently ignored. It is now honored. This is much the largest group:
  699 files (14.2%), worst 5.8% peak change, median 0.11%.
- Channel pressure now reaches vibrato depth through SF2 default modulator 3, having previously had
  no effect whatsoever.
- Controller data bytes are masked to seven bits. A malformed file can deliver a larger one - the
  corpus has files that land 255 in CC11 - and unmasked that made `expression` 1.99 rather than at
  most 1, which the old `(volume * expression)^2` channel gain turned into a 2.4x boost that
  clipped. Together with channel pressure this accounts for 54 files (1.1%), median 1.0%, worst
  65.8% - and where it is large it is because the file was previously being played far too loud.

The same corpus rendered through a stripped GeneralUser GS moves 119 files (2.4%).

That 84.7% are unchanged, and that every file which did move is attributable to one of three named
causes, is what establishes that the default modulator table reproduces the hardcoded controller
handling it replaced rather than merely resembling it. The comparison is by level rather than
bit-for-bit on purpose: the same curve expressed as `10^(0.05 * -0.1 * 960 * concave(v))` instead of
`v^2` agrees to 1e-14 in f64 but not in the last bits of f32, and codegen alone already moves those
- the same source renders differently under `--release` than under `cargo test`.

For the other half of the question, that honoring a font's own modulators actually does something:
99.7% of the corpus sample renders differently through GeneralUser GS with its modulators honored
than with them stripped, and on its grand piano the measured brightness of a note climbs
monotonically from velocity 16 to 127 by a factor of 2.08, against 1.11 and non-monotonic with the
modulators removed. That is the 906 velocity-to-filter-cutoff modulators the font ships.

Rendering costs about **7.6% more** than before (41.9s against 45.1s for 150 corpus files, three
alternating runs, spread under 0.3%).

Fixed along the way:

- `BiQuadFilter::set_low_pass_filter` divides by `1 + 6 * (resonance - 1)`, which is zero at about
  -1.58 dB. Generators alone cannot reach it, but modulators can, and FluidR3_GM ships forty at
  amount -470. The NaN would have propagated into the reverb and chorus comb filters, which are IIR
  with persistent state, and the output would never have recovered. Resonance is now clamped to the
  SF2 generator range, as are attenuation, pan, the tremolo depth and the pitch depths - the last
  two because `modLfoToVolume` is an exponent that overflows to infinity well inside the range a
  modulator can express.

Rendering the corpus sample through all six test fonts - 29,448 files in total, including the
FluidR3_GM case above - produced no NaN or infinity anywhere in the output, and no increase in the
number of files that fail to parse.

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
