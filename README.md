# RustySynth

RustySynth is a SoundFont MIDI synthesizer written in pure Rust, ported from [MeltySynth](https://github.com/sinshu/meltysynth).



## This fork

This is a fork of [sinshu/rustysynth](https://github.com/sinshu/rustysynth). It tracks upstream and
adds what a karaoke machine playing a large, uncurated corpus of MIDI and `.kar` files needs from a
SoundFont synthesizer. The crate keeps its zero dependencies and the published API is additive.

* **SF2 modulators are implemented.** The `pmod` and `imod` chunks were read and discarded, so no
  modulator was heard at all - which is not neutral between banks, since the ones whose design leans
  hardest on modulators lose the most. GeneralUser GS ships 2,257 of them.
* **A SoundFont with one bad record loads without it**, rather than not loading at all. Four of
  fifteen surveyed banks were refused over a single defective record; in one case one `shdr` out of
  5,007. `SoundFont::get_warnings()` says what was dropped. A font with no playable region left is
  still an error.
* **The pitch path is corrected.** `scaleTuning` no longer scales pitch bend, vibrato or channel
  tune, so a fixed-pitch region is not deaf to the pitch wheel; Roland GS drum pitch (NRPN 18H) is
  honored per key; and the RPN/NRPN selectors and data entries are masked to seven bits, where an
  out-of-range byte used to detune a channel for the rest of a file.
* **MIDI channel mode messages are honored.** CC126 puts a channel monophonic with last-note
  priority, CC127 returns it to polyphonic, and CC124-127 all act as All Notes Off. Karaoke files
  write monophonic leads as slurred pairs and declare the channel mono; rendered polyphonically each
  slur is a dyad.
* **A hold pedal lift is counted rather than read as a position**, so a pedal that lifts and presses
  again inside one render block still releases the voices it was holding. Sequencers write a
  re-pedal as a pedal-up and a pedal-down on the same tick, and a part left stacking voices climbs
  until it masks the arrangement.
* **MIDI file parsing fixes**, and a round of hardening in the SoundFont reader: a system real-time
  byte no longer desynchronises a track, running status survives a meta or SysEx event, and several
  malformed-font paths that panicked or wrote past an allocation are bounded.
* **Additional public API**: `get_warnings`, `get_warning_count`, `get_active_voice_count`,
  `get_channel_pitch_bend_range`, `get_channel_tune`, `get_channel_key_tune`, and
  `SynthesizerSettings::reverb_send_scale` / `chorus_send_scale`.

To depend on this fork rather than the published crate, take it from git. `Cargo.lock` then pins the
exact commit, so a build is as reproducible as one against the registry:

```
rustysynth = { git = "https://github.com/rrgmc/rustysynth", branch = "custom" }
```

[`FORK_CHANGES.md`](FORK_CHANGES.md) is the short account against upstream and
[`CHANGELOG.md`](CHANGELOG.md) the full one, with the measurements each change was made on.


## Features

* Suitable for both real-time and offline synthesis.
* Supports standard MIDI files with additional features including dynamic tempo changing.
* No dependencies other than the standard library.



## Demo

This is a demo video to show the synthesizer running on [rust-sfml](https://github.com/jeremyletang/rust-sfml) in real-time.

https://www.youtube.com/watch?v=o9rPTJIPmVk

[![Youtube video](rustysynth-yt.png)](https://www.youtube.com/watch?v=o9rPTJIPmVk)



## Installation

RustySynth is available on [crates.io](https://crates.io/crates/rustysynth):

```
cargo add rustysynth
```



## Examples

Here are some example codes.

> [!NOTE]
> Each example omits the `use` statements.
> For full code, see the respective links.

[An example code to synthesize a simple chord:](example/src/main.rs#L15)

```rust
// Load the SoundFont.
let mut sf2 = File::open("TimGM6mb.sf2").unwrap();
let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

// Create the synthesizer.
let settings = SynthesizerSettings::new(44100);
let mut synthesizer = Synthesizer::new(&sound_font, &settings).unwrap();

// Play some notes (middle C, E, G).
synthesizer.note_on(0, 60, 100);
synthesizer.note_on(0, 64, 100);
synthesizer.note_on(0, 67, 100);

// The output buffer (3 seconds).
let sample_count = (3 * settings.sample_rate) as usize;
let mut left: Vec<f32> = vec![0_f32; sample_count];
let mut right: Vec<f32> = vec![0_f32; sample_count];

// Render the waveform.
synthesizer.render(&mut left[..], &mut right[..]);
```

[Another example code to synthesize a MIDI file:](example/src/main.rs#L41)

```rust
// Load the SoundFont.
let mut sf2 = File::open("TimGM6mb.sf2").unwrap();
let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

// Load the MIDI file.
let mut mid = File::open("flourish.mid").unwrap();
let midi_file = Arc::new(MidiFile::new(&mut mid).unwrap());

// Create the MIDI file sequencer.
let settings = SynthesizerSettings::new(44100);
let synthesizer = Synthesizer::new(&sound_font, &settings).unwrap();
let mut sequencer = MidiFileSequencer::new(synthesizer);

// Play the MIDI file.
sequencer.play(&midi_file, false);

// The output buffer.
let sample_count = (settings.sample_rate as f64 * midi_file.get_length()) as usize;
let mut left: Vec<f32> = vec![0_f32; sample_count];
let mut right: Vec<f32> = vec![0_f32; sample_count];

// Render the waveform.
sequencer.render(&mut left[..], &mut right[..]);
```

[Yet another example code to synthesize a MIDI file in real-time with the TinyAudio crate:](https://github.com/sinshu/rustysynth/blob/tinyaudio/workspace/src/main.rs)

```rust
// Setup the audio output.
let params = OutputDeviceParameters {
    channels_count: 2,
    sample_rate: 44100,
    channel_sample_count: 4410,
};

// Buffer for the audio output.
let mut left: Vec<f32> = vec![0_f32; params.channel_sample_count];
let mut right: Vec<f32> = vec![0_f32; params.channel_sample_count];

// Load the SoundFont.
let mut sf2 = File::open("TimGM6mb.sf2").unwrap();
let sound_font = Arc::new(SoundFont::new(&mut sf2).unwrap());

// Load the MIDI file.
let mut mid = File::open("flourish.mid").unwrap();
let midi_file = Arc::new(MidiFile::new(&mut mid).unwrap());

// Create the MIDI file sequencer.
let settings = SynthesizerSettings::new(params.sample_rate as i32);
let synthesizer = Synthesizer::new(&sound_font, &settings).unwrap();
let mut sequencer = MidiFileSequencer::new(synthesizer);

// Play the MIDI file.
sequencer.play(&midi_file, false);

// Start the audio output.
let _device = run_output_device(params, {
    move |data| {
        sequencer.render(&mut left[..], &mut right[..]);
        for (i, value) in left.iter().interleave(right.iter()).enumerate() {
            data[i] = *value;
        }
    }
})
.unwrap();

// Wait for 10 seconds.
std::thread::sleep(std::time::Duration::from_secs(10));
```



## Todo

* __Wave synthesis__
    - [x] SoundFont reader
    - [x] Waveform generator
    - [x] Envelope generator
    - [x] Low-pass filter
    - [x] Vibrato LFO
    - [x] Modulation LFO
* __MIDI message processing__
    - [x] Note on/off
    - [x] Bank selection
    - [x] Modulation
    - [x] Volume control
    - [x] Pan
    - [x] Expression
    - [x] Hold pedal
    - [x] Program change
    - [x] Pitch bend
    - [x] Tuning
* __Effects__
    - [x] Reverb
    - [x] Chorus
* __Other things__
    - [x] Standard MIDI file support
    - [x] MIDI file loop extension support
    - [x] Performace optimization



## License

RustySynth is available under [the MIT license](LICENSE.txt).
