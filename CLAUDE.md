# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

RustySynth is a SoundFont (SF2) MIDI synthesizer written in pure Rust, ported from the C#
project [MeltySynth](https://github.com/sinshu/meltysynth). The library crate has **zero
dependencies** — only the standard library — and that is a deliberate selling point. Do not add
one without asking.

## Workspace layout

Cargo workspace with three members:

| Crate | Role |
| --- | --- |
| `rustysynth/` | The published library (v1.3.6, edition 2021). All real code lives here. |
| `rustysynth_test/` | SoundFont-parser regression tests. Note it is a **lib crate** with `#[test]`s in `src/*_test.rs`, not a `tests/` directory. |
| `example/` | Binary demo that renders `.pcm` files. |

## Commands

```sh
cargo build                                    # or --release
cargo test -p rustysynth                       # the tests that run with no setup (3 tests)
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

`SoundFont::new` (`rustysynth/src/soundfont.rs:34`) walks the RIFF container:
`SoundFontInfo` → `SoundFontSampleData` → `SoundFontParameters` → `sanity_check`
(`soundfont.rs:68`), which rejects out-of-range sample and loop points — several past CVE-ish
panics were fixed by tightening it. SoundFont3 is explicitly rejected with
`SoundFontError::UnsupportedSampleFormat`.

**SF2 modulators are not implemented.** The `pmod` and `imod` chunks are read and thrown away
(`soundfont_parameters.rs:58,62`); only generators drive synthesis.

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

The per-voice chain in `Voice::process` (`voice.rs:191`) is **oscillator → biquad low-pass →
gain/pan**. Critically, the envelopes and LFOs advance **once per block (control rate), not per
sample** — anything you add that modulates per-sample breaks that model.

### Channels

Exactly 16 `Channel`s, index 9 forced to percussion (bank 128). Worth knowing: `reset()` leaves
`reverb_send` at **40, not 0** (`channel.rs:63`), and `reset_all_controllers()` deliberately
preserves volume, pan, and sends. NRPN data entry is accepted but ignored.

### MIDI file playback

`MidiFileSequencer::render` (`midifile_sequencer.rs:82`) mirrors the synthesizer's block loop, so
**MIDI events are quantized to the block boundary** (~1.45 ms at 44.1 kHz with a 64-sample
block). `MidiFile` pre-flattens the file into parallel `messages`/`times` arrays with tempo
already resolved to absolute seconds, so no tempo math happens at render time; `set_speed` scales
the wall-clock advance rather than rewriting tempo. Loop markers are detected at parse time from
the loop-type-specific CCs (`MidiFileLoopType`: RPG Maker CC111, Incredible Machine CC110/111,
Final Fantasy CC116/117, or an explicit tick).

## Conventions

- **Accessors, not public fields.** Types expose `get_xxx()` over `pub(crate)` fields — a
  MeltySynth port artifact. Match it rather than exposing fields directly.
- `#![allow(dead_code)]` sits at the top of most source files (33 of 42); it is intentional, since
  much of the ported API surface is unused internally.
- Public types are `#[non_exhaustive]` and derive `Debug`. Adding a variant or field is
  non-breaking; construct them through their constructors.
- Error enums (`SoundFontError`, `MidiFileError`, `SynthesizerError` in `error.rs`) do **no heap
  allocation** — they carry `FourCC` and integers, never `String`. Keep new variants allocation-free.
- Audio math is `f32`; `f64` is reserved for a few spots in `soundfont_math.rs` and for time in
  the sequencer.
- `SynthesizerSettings::validate` (`synthesizer_settings.rs:38`) enforces sample rate
  16000–192000, block size 8–1024, polyphony 8–256.
- Update `CHANGELOG.md` for user-visible changes; it is organized by released version.
