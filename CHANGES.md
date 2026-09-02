# Changes on `feat/lenient-soundfont-loading`

Summary of what this branch changes relative to `main` (23 commits, 47 files): SF2 modulator
support, lenient SoundFont loading, a round of parser hardening and two MIDI parsing fixes. The
crate goes from 1.3.6 to 1.5.0 and still has zero dependencies. `CHANGELOG.md` carries the full
account with the measurements; this is the short version.

## SF2 modulators

- **`pmod` and `imod` are parsed and resolved onto regions** instead of being read and discarded, so
  synthesis is no longer driven by generators alone. GeneralUser GS alone ships 2,257 modulators, and its
  entire velocity-dependent brightness was missing.
- Channel pressure (aftertouch) and polyphonic key pressure are received; channel pressure drives
  vibrato depth through SF2 default modulator 3 and previously had no effect at all.
- Added `SynthesizerSettings::reverb_send_scale` and `chorus_send_scale`, for fonts that cap their
  own sends well below full scale.
- Deliberate deviations from SF2 2.04, all chosen to preserve existing output: default modulator 2
  is omitted (as FluidSynth does), default modulator 10 is handled natively in `voice.rs`, the send
  defaults use amount 1000 rather than 200, and linked modulators and non-identity transforms are
  dropped at load time.
- Resonance, attenuation, pan and the LFO depths are clamped to their generator ranges. A modulator
  could otherwise drive resonance to where `BiQuadFilter` divides by zero, and the NaN would persist
  in the reverb and chorus state — FluidR3_GM ships forty that reach it.

## Lenient SoundFont loading

- **A bad record now costs that record, not the file.** An out-of-range region, a zone naming no
  sample, a preset with no usable zone, an unknown chunk id: each drops what failed and loading
  continues. Every check that stops the oscillator indexing outside the wave data is kept.
- What was dropped is reported. `SoundFont::get_warnings()` returns `SoundFontWarning` values naming
  the instrument, the region and which condition it failed; `get_warning_count()` gives the total,
  since the kept list is capped at 64.
- A font with no playable region left anywhere is still `Err(SanityCheckFailed)`.
- Two asymmetries are load-bearing: an instrument with no zones is kept empty while a preset with no
  usable zone is dropped (preset regions address instruments by position), and a zone is only a
  region if it carries the terminating generator SF2 7.7 requires, rather than falling through to
  sample 0.

## Parser hardening

- `read_wave_data` allocated `size / 2` samples then read `size` bytes over that allocation through
  an unsafe slice. It now reads in blocks with no `unsafe` and reserves fallibly.
- `discard_data` allocated and zeroed whatever a chunk header claimed before reading any of it.
- `Zone::new` indexed straight into `pgen`/`igen`, so a bag record pointing past the end panicked.
- RIFF's pad byte after an odd-sized chunk is consumed, and `ifil`/`iver` respect their declared
  size, so neither desynchronises the stream.
- `ListContainsUnknownId` names the list it came from rather than always claiming `INFO`, and
  `SubChunkNotFound` reports the ids actually on disk.

## MIDI file parsing

- A running-status event following a meta or SysEx event was decoded with the wrong status byte and
  silently discarded.
- A track with no end-of-track meta event was parsed past the end of its own chunk.

## New public API

`Modulator`, `ModulatorSource`, `SoundFontWarning` and `RegionDefect` are exported;
`SoundFont::get_warnings()` and `get_warning_count()`, `PresetRegion::get_modulators()` and
`InstrumentRegion::get_modulators()`, and the two send-scale settings are new.

## Tests and tooling

- `rustysynth_regress` is a new, unpublished workspace member: `load`, `census`, `strip-mods`,
  `render`, `sample`, `compare` and `probe`. It renders a MIDI corpus and reduces each file to one
  line so two builds can be compared, against fonts too large to commit.
- Seven committed fixtures in `samples/` — six carrying one malformed record each, one carrying
  modulators — built by `make_test_malformed.py` and `make_test_modulators.py`.
- A golden-render test in `rustysynth_test`, compared by tolerance rather than by hash, plus
  channel-state and modulator-merge unit tests in the library crate.

## Verification on record

600 files sampled from a 616,563 file corpus render identically through GeneralUser GS and
FluidR3_GM against the previous build; six real banks load with zero warnings; 84.7% of a 4,973 file
corpus is unchanged under the default modulator table, and every file that moved is attributable to
one of three named causes. Rendering costs about 7.6% more. Method and figures are in `CHANGELOG.md`.

One caveat: none of the four unopenable banks the leniency was written for were available to test
against, so what is verified is the mechanism and the absence of regression, not that those specific
files now play.
