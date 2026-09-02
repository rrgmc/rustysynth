#![allow(dead_code)]

/// A single channel message from a MIDI file, with its time already resolved.
///
/// `MidiFile` flattens its tracks into one time-ordered sequence with the tempo
/// map already applied, and `MidiFile::get_events` hands that sequence out.
/// This is what a host needs to drive `Synthesizer::process_midi_message`
/// itself rather than through `MidiFileSequencer` - to mute a channel, to
/// render one part in isolation, or to record what a file asked for.
///
/// Only channel messages appear. Tempo changes are already resolved into
/// `time`, and the loop markers and end-of-track records the sequencer uses are
/// not channel messages, so neither is reported here.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct MidiEvent {
    pub(crate) time: f64,
    pub(crate) channel: u8,
    pub(crate) command: u8,
    pub(crate) data1: u8,
    pub(crate) data2: u8,
}

impl MidiEvent {
    /// Gets the time of the message in seconds from the start of the file.
    pub fn get_time(&self) -> f64 {
        self.time
    }

    /// Gets the channel the message is addressed to, from 0 to 15.
    pub fn get_channel(&self) -> i32 {
        self.channel as i32
    }

    /// Gets the command nibble of the status byte, such as `0x90` for note on.
    pub fn get_command(&self) -> i32 {
        self.command as i32
    }

    /// Gets the first data byte, such as the key of a note-on.
    pub fn get_data1(&self) -> i32 {
        self.data1 as i32
    }

    /// Gets the second data byte, such as the velocity of a note-on. Zero for
    /// the two commands that carry only one.
    pub fn get_data2(&self) -> i32 {
        self.data2 as i32
    }
}
