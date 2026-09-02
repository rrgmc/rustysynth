# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

RustySynth is a SoundFont (SF2) MIDI synthesizer written in pure Rust, ported from the C#
project [MeltySynth](https://github.com/sinshu/meltysynth). The library crate has **zero
dependencies** — only the standard library — and that is a deliberate selling point. Do not add
one without asking.

## Workspace layout

Cargo workspace with four members:

| Crate | Role |
| --- | --- |
| `rustysynth/` | The published library (v1.4.0, edition 2021). All real code lives here. |
| `rustysynth_test/` | SoundFont-parser regression tests. Note it is a **lib crate** with `#[test]`s in `src/*_test.rs`, not a `tests/` directory. |
| `rustysynth_regress/` | Unpublished verification harness. Renders a MIDI corpus against SoundFonts too large to commit and reduces each file to one line, so two builds can be compared. Also strips a font's modulator chunks, which is how the "does this still sound the same?" control font is made — none of the available fonts ships without modulators. `diagnose` goes the other way, taking one file apart: `stems` renders each channel as its own WAV, `notes` reports what every note-on resolved to and, in cents, the offset from equal temperament the font asks for (intent, not evidence of a synthesis fault), `voices` reports the polyphony the file wants. |
| `example/` | Binary demo that renders `.pcm` files. |

## Commands

```sh
cargo build                                    # or --release
cargo test -p rustysynth                       # the tests that run with no setup (40 tests)
cargo test -p rustysynth test_load_reject_sf3  # a single test
cargo clippy --all-targets                     # currently clean; keep it that way
cargo fmt
```

### Test assets are gitignored and absent

`.gitignore` excludes `*.sf2`, `*.sf3`, `*.mid`, and `*.pcm` (with a `!samples/*` exception), so
a bare `cargo test` **fails** on 8 tests with `NotFound`. This is expected in a fresh checkout,
not a regression:

- `rustysynth_test` opens `TimGM6mb.sf2` and `GeneralUser GS MuseScore v1.442.sf2` from the
  **workspace root** (each test does `CARGO_MANIFEST_DIR` then `.pop()`, e.g.
  `rustysynth_test/src/timgm6mb_info_test.rs:9-11`). Drop both files there to run them.
- `cargo run -p example` needs `TimGM6mb.sf2` and `flourish.mid` in the CWD.

The tests that run unaided are inside the library crate and use the committed fixtures in
`samples/`: `rustysynth/src/soundfont.rs:127` and `rustysynth/src/midifile.rs:337`. Prefer adding
new tests there, with a fixture in `samples/`, over adding to `rustysynth_test`.

## Architecture

### SoundFont loading

`SoundFont::new` (`rustysynth/src/soundfont.rs`) walks the RIFF container:
`SoundFontInfo` → `SoundFontSampleData` → `SoundFontParameters` → `drop_unplayable_regions`, which
checks every region's out-of-range sample and loop points — several past CVE-ish panics were fixed
by tightening it. SoundFont3 is explicitly rejected with
`SoundFontError::UnsupportedSampleFormat`.

**Loading is lenient, and says what it dropped.** As of v1.5.0 a bad record costs that record, not
the file: an out-of-range region, a zone naming no sample, a preset with no usable zone, an unknown
chunk id. Each one is recorded, and `SoundFont::get_warnings()` returns them
(`soundfont_warning.rs`), with `get_warning_count()` for the total since the kept list is capped at
64. A font with **no** playable region left anywhere is still `Err(SanityCheckFailed)`.

Two asymmetries in that are load-bearing and easy to undo by accident:

- An instrument with no zones is **kept, empty**; a preset with no usable zone is **dropped**.
  `PresetRegion` resolves instruments by *position* (`preset_region.rs`), so removing an instrument
  repoints every later preset at the wrong one. Presets are found by bank and patch, so an empty
  one would be found, play silence, and suppress the bank-0 fallback in `synthesizer.rs`.
- A zone is only a region if it actually carries the terminating generator SF2 7.7 requires —
  `sampleID` for an instrument zone, `instrument` for a preset zone. Without that check `gs[]`'s
  zero-initialized slot silently binds the zone to sample 0.

Before changing any of this, re-run the corpus comparison: `rustysynth_regress render` on both
builds then `compare`, which must report only rows that failed in both. Dropping a region that a
font actually plays is the failure mode, and unit tests will not catch it.

**SF2 modulators are implemented** (`modulator.rs`, `modulator_source.rs`,
`default_modulators.rs`). `pmod` and `imod` are parsed into `Zone`, then merged onto regions by the
SF2 9.5.4 rule - a modulator identical in source, destination and amount source *replaces* the one
already there rather than adding to it, which is how a font overrides a default. Instrument regions
carry two lists: `modulators` (what the font says, exposed by `get_modulators()`) and
`resolved_modulators` (that merged over the defaults, used at note-on).

Deviations from the spec are deliberate and listed in `CHANGELOG.md` under v1.4.0: default
modulator 2 is omitted, default 10 (pitch bend) is handled natively in `voice.rs` rather than
through the engine, and the send defaults use amount 1000 rather than 200. Linked modulators and
non-identity transforms are dropped at load time - the latter because preset and instrument amounts
are *summed* at voice start, which is only equivalent to evaluating them separately while the
transform is linear.

### Region resolution (preset + instrument → voice parameters)

`PresetRegion` and `InstrumentRegion` each flatten their zones into a dense
`gs: [i16; GeneratorType::COUNT]` array indexed by generator number — global zone applied first,
then the local zone, last write wins (`preset_region.rs:61`, `instrument_region.rs:92`).
Instrument regions start from the SF2 spec defaults; preset regions start at zero because preset
generators are *offsets*.

`RegionPair` (`region_pair.rs:10`) is a borrowed view of one `(preset, instrument)` pair, and its
private `gs()` (`region_pair.rs:20`) implements the combination rule: **`preset.gs[i] +
instrument.gs[i]`**. Sample addressing, `sample_modes`, `exclusive_class`, and `root_key` are
*not* summed — they come from the instrument only. `RegionEx` (`region_ex.rs`) adapts a
`RegionPair` into the start arguments for the oscillator, envelopes, and LFOs.

**`scale_tuning` multiplies the key interval only.** `Oscillator::pitch_ratio` splits its `pitch`
argument into `scale_tuning/100 × (key − root_key)` plus the modulation — the LFOs, the modulation
envelope, the channel tune, the pitch bend and the per-key drum tune — added in real semitones.
This is the one place the port deliberately leaves MeltySynth, which scaled the whole sum: SF2
2.04 8.1.2 defines the generator as the influence of *key number*, and a fixed-pitch region
(`scale_tuning` 0, which several fonts ship) went completely deaf to pitch bend and vibrato under
the old form. `Oscillator` therefore has to be told the note's `key` at `start()` as well as
`root_key`; both are fixed for the voice's life.

### Rendering

`Synthesizer::render` (`synthesizer.rs:334`) is a pull loop that decouples the caller's buffer
length from the fixed internal `block_size` (default 64): it renders a block whenever the read
cursor is exhausted and memcpy's out of `block_left`/`block_right`. `render_block`
(`synthesizer.rs:362`) then, in order: processes all active voices, accumulates them into the
stereo block, runs chorus (stereo in/out), and runs reverb (mono in, stereo out, Freeverb-style).

`write_block` (`synthesizer.rs:470`) is the anti-zipper stage — it interpolates between each
voice's `previous_*` and `current_*` mix gains, which is why `Voice` carries gain pairs.

### Voices

`VoiceCollection` is a fixed pool sized by `maximum_polyphony`; active voices are the prefix
`[0..active_voice_count]` and are retired by O(1) swap-remove. `request_new`
(`voice_collection.rs:28`) allocates in three tiers: reuse a voice with the same non-zero
exclusive class (drum choke) → take a free slot → steal the lowest `priority()`, which is
envelope-stage ranked so releasing voices are stolen before attacking ones.

The per-voice chain in `Voice::process` is **oscillator → biquad low-pass → gain/pan**. Critically,
the envelopes, LFOs *and modulators* advance **once per block (control rate), not per sample** —
anything you add that modulates per-sample breaks that model. No modulator source is per-sample
either: every one is per-voice-static (velocity, key) or per-channel (CC, pressure, pitch wheel).

Every synthesis parameter is the sum of three arrays indexed by generator number: `gen_cb` (preset
plus instrument generators), `static_cb` (modulators whose sources cannot change) and `dyn_cb`
(re-evaluated each block). They are kept apart because the legacy Polyphone-derived scale factors —
`0.4 ×` generator attenuation, `0.5 ×` filter Q — apply to the generator *only*; scaling the
velocity curve by 0.4 as well would flatten every SoundFont's dynamics by 60%.

Two traps live here. Dynamic attenuation goes to `mix_gain`, never `note_gain`: `note_gain` below
`NON_AUDIBLE` retires the voice permanently, so folding CC7/CC11 in would destroy every voice on a
channel whenever an expression pedal swept to zero. And resonance must stay clamped to `[0, 960]`
cB — `bi_quad_filter.rs:58` divides by `1 + 6·(resonance − 1)`, zero at about −1.58 dB, and the
resulting NaN poisons the IIR reverb and chorus state permanently. FluidR3_GM really does ship
modulators that reach it.

### Channels

Exactly 16 `Channel`s, index 9 forced to percussion (bank 128). Worth knowing: `reset()` leaves
`reverb_send` at **40, not 0** (`channel.rs:63`), and `reset_all_controllers()` deliberately
preserves volume, pan, and sends.

**One NRPN is honored, the rest are accepted and dropped.** GS 18H, drum instrument pitch coarse,
lands in `Channel::key_tune` as a per-key semitone offset and is read at `voice.rs`'s
`channel_pitch_change`; `get_key_tune` gates on `bank_number >= 128`, because the same key numbers
are pitches the font already tunes on a melodic part. Everything else GS defines there - vibrato
rate, TVF cutoff, envelope times, per-key level and pan - still sets `last_data_type` and discards
its value, which is what keeps a data entry following an NRPN from being read as pitch bend
sensitivity (`channel.rs`, `data_entry_coarse`).

Both RPN selector bytes are needed to select a parameter: `rpn` is packed MSB-over-LSB and starts at
-1, which reads as the null parameter 127:127, so a file sending only CC 101 = 0 selects RPN 0:127
and its data entry is dropped. That matches hardware, where the two are independent registers that
power up null, and diverges from FluidSynth, which zeroes both. Real GS files send both bytes.

### MIDI file playback

`MidiFileSequencer::render` (`midifile_sequencer.rs:82`) mirrors the synthesizer's block loop, so
**MIDI events are quantized to the block boundary** (~1.45 ms at 44.1 kHz with a 64-sample
block). `MidiFile` pre-flattens the file into parallel `messages`/`times` arrays with tempo
already resolved to absolute seconds, so no tempo math happens at render time; `set_speed` scales
the wall-clock advance rather than rewriting tempo. Loop markers are detected at parse time from
the loop-type-specific CCs (`MidiFileLoopType`: RPG Maker CC111, Incredible Machine CC110/111,
Final Fantasy CC116/117, or an explicit tick).

`MidiFile::get_events` (`midifile.rs`) hands that flattened sequence out as `MidiEvent`s, which is
how a host drives `Synthesizer::process_midi_message` itself instead of through the sequencer —
what `rustysynth_regress diagnose stems` uses to render one channel at a time. Only channel
messages appear; tempo is already in the time and the loop markers are not channel messages.

**Every status byte in `read_track`'s `match` needs a length, or the track desynchronises.** The
final arm reads two data bytes unconditionally, so a system byte that reaches it eats the events
after it and the rest of the part comes out as wrong notes rather than as an error. `0xF1`/`0xF3`
take one, `0xF2` takes two, and `0xF4`–`0xF6`/`0xF8`–`0xFE` take none. Only the channel-message arm
assigns `last_status`; a meta or SysEx event deliberately leaves the previous channel status
standing, against the spec, because karaoke writers put a lyric between a note-on and its
running-status successor.

## Conventions

- **Accessors, not public fields.** Types expose `get_xxx()` over `pub(crate)` fields — a
  MeltySynth port artifact. Match it rather than exposing fields directly.
- `#![allow(dead_code)]` sits at the top of most source files (33 of 42); it is intentional, since
  much of the ported API surface is unused internally.
- Public types are `#[non_exhaustive]` and derive `Debug`. Adding a variant or field is
  non-breaking; construct them through their constructors.
- Error enums (`SoundFontError`, `MidiFileError`, `SynthesizerError` in `error.rs`) do **no heap
  allocation** — they carry `FourCC` and integers, never `String`. Keep new variants allocation-free.
  `SoundFontWarning` follows the same rule for the same reason.
- Audio math is `f32`; `f64` is reserved for a few spots in `soundfont_math.rs` and for time in
  the sequencer.
- `SynthesizerSettings::validate` (`synthesizer_settings.rs:38`) enforces sample rate
  16000–192000, block size 8–1024, polyphony 8–256.
- Update `CHANGELOG.md` for user-visible changes; it is organized by released version.
