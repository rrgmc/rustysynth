# Changes on `feat/lenient-soundfont-loading`

Summary of what this branch changes relative to `main` (33 commits, 53 files): SF2 modulator
support, lenient SoundFont loading, two fixes to the pitch path, the MIDI channel mode messages, a
round of hardening and three MIDI parsing fixes. The crate goes from 1.3.6 to 1.5.0 and still has zero dependencies. `CHANGELOG.md`
carries the full account with the measurements; this is the short version.

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

## Hardening

- `read_wave_data` allocated `size / 2` samples then read `size` bytes over that allocation through
  an unsafe slice. It now reads in blocks with no `unsafe` and reserves fallibly.
- `discard_data` allocated and zeroed whatever a chunk header claimed before reading any of it.
- `Zone::new` indexed straight into `pgen`/`igen`, so a bag record pointing past the end panicked.
- RIFF's pad byte after an odd-sized chunk is consumed, and `ifil`/`iver` respect their declared
  size, so neither desynchronises the stream.
- `ListContainsUnknownId` names the list it came from rather than always claiming `INFO`, and
  `SubChunkNotFound` reports the ids actually on disk.
- The RPN/NRPN selectors and both data entries are masked to seven bits - the last controllers a raw
  value still reached, and the ones where an out-of-range byte does lasting rather than momentary
  damage. 255 into CC 6 under RPN 0 asked for a 255 semitone pitch bend range; under RPN 1 it
  detuned the channel for the rest of the file. The packed selector also overflowed from 256 up.
- The oscillator's loop wrap subtracts until the position is inside the loop rather than once per
  output sample, which is unbounded for a short loop played far above its root key and would
  eventually have indexed off the end of the wave data.

## Pitch and tuning

- **`scaleTuning` no longer scales pitch bend, vibrato or channel tune.** SF2 2.04 8.1.2 defines the
  generator as the degree to which *MIDI key number* influences pitch, so the interval from the root
  key is scaled and everything modulating the note is added in real semitones. What the old form
  ruined is a fixed-pitch region - `scaleTuning` 0, a font saying "every key plays this sample
  untransposed" - because scaling the modulation by zero too left it deaf to the pitch wheel and to
  vibrato. GeneralUser GS ships eight such regions, SGM-V2.01 ships 174; a region at the default 100
  is unaffected, which is nearly all of them.
- **Roland GS drum pitch (NRPN 18H) is honored**, as a per-key semitone offset in
  `Channel::key_tune`, gated on the channel holding a drum kit. Every NRPN value used to be
  discarded, which is right for the ones this synthesizer has no parameter for and wrong for this
  one: on a drum part each key is a separate instrument, so retuning one tom without moving the
  snare beside it is not something a channel-wide tune can express. The karaoke file this came from
  retunes a kick, a snare, a tom and both agogo bells, and dropping it left 915 of that file's 1,982
  percussion notes at the wrong pitch - the agogos nine semitones sharp, and the only pitched
  percussion in the arrangement. Every other NRPN is still accepted and dropped.
- One limitation written down rather than fixed: a dynamic modulator on `coarseTune`, `fineTune` or
  `scaleTuning` is re-evaluated every block and then never read, because the oscillator latches all
  three out of the note-on snapshot. No bank in the cache asks for it.

## MIDI file parsing

- A running-status event following a meta or SysEx event was decoded with the wrong status byte and
  silently discarded.
- A track with no end-of-track meta event was parsed past the end of its own chunk.
- A system common or real-time status byte desynchronised the rest of the track. `0xF0`, `0xF7` and
  `0xFF` were handled and everything else fell through to the channel-message arm, which reads two
  data bytes unconditionally, so a single `0xF8` clock or `0xFE` active sensing byte shifted every
  delta time, status byte and key after it - the part came out as wrong notes rather than as an
  error. It also stranded the running status, so the following events decoded as `0xF0` and were
  dropped.

## MIDI channel modes

- **CC124-127 were ignored outright.** CC126 now puts a channel monophonic, CC127 returns it to
  polyphonic, and all four act as All Notes Off as the spec requires of every mode message.
  Karaoke files write monophonic leads as slurs, each note-on landing before the previous note-off,
  and declare the channel mono so the receiver collapses them; played polyphonically each slur is a
  brief second or seventh instead. `EBONYIVO.KAR` declares mono on five channels and has seventeen
  such overlaps, 31 to 123 ms each.
- **Last-note priority**, because that file needs it: it nests a 60 ms grace note inside an 800 ms
  sustained one, and stopping the old note without falling back to the key still held loses the
  sustained note entirely. The held keys are a fixed sixteen-entry array on the channel, since
  note-on may not allocate.
- Omni Off and Omni On deliberately do not select mono or poly - the two are orthogonal bits of the
  MIDI mode, so a file sending the conventional CC124 + CC126 pair for mode 4 would otherwise be
  forced back to poly. Mono mode survives Reset All Controllers, which resets controllers and not
  channel modes. The previous note is released rather than killed, so a release tail of the older
  pitch remains; hardware reassigns one voice and has no overlap at all, which would mean changing a
  sounding voice's key.

## New public API

`Modulator`, `ModulatorSource`, `SoundFontWarning` and `RegionDefect` are exported;
`SoundFont::get_warnings()` and `get_warning_count()`, `PresetRegion::get_modulators()` and
`InstrumentRegion::get_modulators()`, and the two send-scale settings are new.

## Tests and tooling

- `rustysynth_regress` is a new, unpublished workspace member: `load`, `census`, `strip-mods`,
  `render`, `sample`, `compare`, `probe` and `diagnose`. It renders a MIDI corpus and reduces each
  file to one line so two builds can be compared, against fonts too large to commit.
- `diagnose` goes the other way and takes one file apart: `stems` renders each channel as its own
  WAV, `notes` reports what every note-on resolved to and its tuning error in cents, `voices`
  reports the polyphony the file wants. It is what localised the off-tone notes to the drum part,
  and it needs `MidiFile::get_events`, also new, to drive `process_midi_message` per channel.
- Seven committed fixtures in `samples/` — six carrying one malformed record each, one carrying
  modulators — built by `make_test_malformed.py` and `make_test_modulators.py`.
- A golden-render test in `rustysynth_test`, compared by tolerance rather than by hash, plus
  channel-state and modulator-merge unit tests in the library crate.

## Verification on record

600 files sampled from a 616,563 file corpus render identically through GeneralUser GS and
FluidR3_GM against the previous build; six real banks load with zero warnings; 84.7% of a 4,973 file
corpus is unchanged under the default modulator table, and every file that moved is attributable to
one of three named causes. Rendering costs about 7.6% more. Method and figures are in `CHANGELOG.md`.

The pitch work was measured the same way: of 150 files sampled from the corpus and rendered through
GeneralUser GS, 142 are bit-identical and 8 differ - the files that bend a note in a region the font
did not leave at `scaleTuning` 100 - with no NaN and nothing newly failing. The three hardening
fixes above move no row at all. The drum retune was confirmed end to end on a two-file controlled
render rather than inferred: the retuned key moves by -9.00 semitones, the value the file asks for.

The channel modes were verified against the files that actually use them rather than a blind sample:
of 25,000 corpus files, 641 send CC124-127, and rendering all 641 through GeneralUser GS on both
builds moves 50 rows. 45 of those are files that declare mono - the fix doing its work. The other
five are the new All Notes Off on a non-mono mode message, and each loses one or two notes, four of
them a single drum hit on channel 9 and three of them in the first bar. A 150 file random sample is
bit-identical, which is the expected result: only 2.6% of the corpus sends these controllers at all.
`EBONYIVO.KAR` was also checked by ear-equivalent measurement rather than by hash - the older pitch
of each clash drops out and the sustained note the grace note used to kill returns within 1.3 dB of
where it was.

One caveat: none of the four unopenable banks the leniency was written for were available to test
against, so what is verified is the mechanism and the absence of regression, not that those specific
files now play. The other is that no automated check hears anything - the corpus comparison proves
two builds agree, not that either is right.
