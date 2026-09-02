#![allow(dead_code)]

use std::io::Read;

use crate::binary_reader::BinaryReader;
use crate::four_cc::FourCC;
use crate::read_counter::ReadCounter;
use crate::MidiEvent;
use crate::MidiFileError;
use crate::MidiFileLoopType;

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub(crate) enum Message {
    Normal { status: u8, data1: u8, data2: u8 },
    TempoChange { bytes: [u8; 3] },
    LoopStart,
    LoopEnd,
    EndOfTrack,
}

impl Message {
    pub(crate) fn common1(status: u8, data1: u8) -> Self {
        Self::Normal {
            status,
            data1,
            data2: 0,
        }
    }

    pub(crate) fn common2(status: u8, data1: u8, data2: u8, loop_type: MidiFileLoopType) -> Self {
        let command = status & 0xF0;

        if command == 0xB0 {
            match loop_type {
                MidiFileLoopType::RpgMaker if data1 == 111 => {
                    return Message::LoopStart;
                }

                MidiFileLoopType::IncredibleMachine => {
                    if data1 == 110 {
                        return Message::LoopStart;
                    }
                    if data1 == 111 {
                        return Message::LoopEnd;
                    }
                }

                MidiFileLoopType::FinalFantasy => {
                    if data1 == 116 {
                        return Message::LoopStart;
                    }
                    if data1 == 117 {
                        return Message::LoopEnd;
                    }
                }

                _ => (),
            }
        }

        Self::Normal {
            status,
            data1,
            data2,
        }
    }

    pub(crate) fn tempo_change(tempo: i32) -> Self {
        // Truncate to u24
        let bytes = tempo.to_be_bytes()[1..].try_into().unwrap();
        Self::TempoChange { bytes }
    }
}

/// Represents a standard MIDI file.
#[derive(Debug)]
#[non_exhaustive]
pub struct MidiFile {
    pub(crate) messages: Vec<Message>,
    pub(crate) times: Vec<f64>,
}

impl MidiFile {
    /// Loads a MIDI file from the stream.
    ///
    /// # Arguments
    ///
    /// * `reader` - The data stream used to load the MIDI file.
    pub fn new<R: Read>(reader: &mut R) -> Result<Self, MidiFileError> {
        MidiFile::new_with_loop_type(reader, MidiFileLoopType::LoopPoint(0))
    }

    /// Loads a MIDI file from the stream with a specified loop type.
    ///
    /// # Arguments
    ///
    /// * `reader` - The data stream used to load the MIDI file.
    /// * `loop_type` - The type of the loop extension to be used.
    ///
    /// # Remarks
    ///
    /// `MidiFileLoopType` has the following variants:
    /// * `LoopPoint(usize)` - Specifies the loop start point by a tick value.
    /// * `RpgMaker` - The RPG Maker style loop.
    ///   CC #111 will be the loop start point.
    /// * `IncredibleMachine` - The Incredible Machine style loop.
    ///   CC #110 and #111 will be the start and end points of the loop.
    /// * `FinalFantasy` - The Final Fantasy style loop.
    ///   CC #116 and #117 will be the start and end points of the loop.
    pub fn new_with_loop_type<R: Read>(
        reader: &mut R,
        loop_type: MidiFileLoopType,
    ) -> Result<Self, MidiFileError> {
        let chunk_type = BinaryReader::read_four_cc(reader)?;
        if chunk_type != b"MThd" {
            return Err(MidiFileError::InvalidChunkType {
                expected: FourCC::from_bytes(*b"MThd"),
                actual: chunk_type,
            });
        }

        let size = BinaryReader::read_i32_big_endian(reader)?;
        if size != 6 {
            return Err(MidiFileError::InvalidChunkData(FourCC::from_bytes(
                *b"MThd",
            )));
        }

        let format = BinaryReader::read_i16_big_endian(reader)?;
        if !(format == 0 || format == 1) {
            return Err(MidiFileError::UnsupportedFormat(format));
        }

        let track_count = BinaryReader::read_i16_big_endian(reader)? as i32;
        let resolution = BinaryReader::read_i16_big_endian(reader)? as i32;

        let mut message_lists: Vec<Vec<Message>> = Vec::new();
        let mut tick_lists: Vec<Vec<i32>> = Vec::new();

        for _i in 0..track_count {
            let (message_list, tick_list) = MidiFile::read_track(reader, loop_type)?;
            message_lists.push(message_list);
            tick_lists.push(tick_list);
        }

        match loop_type {
            MidiFileLoopType::LoopPoint(loop_point) if loop_point != 0 => {
                let loop_point = loop_point as i32;
                let tick_list = &mut tick_lists[0];
                let message_list = &mut message_lists[0];

                if loop_point <= *tick_list.last().unwrap() {
                    for i in 0..tick_list.len() {
                        if tick_list[i] >= loop_point {
                            tick_list.insert(i, loop_point);
                            message_list.insert(i, Message::LoopStart);
                            break;
                        }
                    }
                } else {
                    tick_list.push(loop_point);
                    message_list.push(Message::LoopStart);
                }
            }
            _ => (),
        }

        let (messages, times) = MidiFile::merge_tracks(&message_lists, &tick_lists, resolution);

        Ok(Self { messages, times })
    }

    fn discard_data<R: Read>(reader: &mut R) -> Result<(), MidiFileError> {
        let size = BinaryReader::read_i32_variable_length(reader)? as usize;
        BinaryReader::discard_data(reader, size)?;
        Ok(())
    }

    fn read_tempo<R: Read>(reader: &mut R) -> Result<i32, MidiFileError> {
        let size = BinaryReader::read_i32_variable_length(reader)?;
        if size != 3 {
            return Err(MidiFileError::InvalidTempoValue);
        }

        let b1 = BinaryReader::read_u8(reader)? as i32;
        let b2 = BinaryReader::read_u8(reader)? as i32;
        let b3 = BinaryReader::read_u8(reader)? as i32;

        Ok((b1 << 16) | (b2 << 8) | b3)
    }

    fn read_track<R: Read>(
        reader: &mut R,
        loop_type: MidiFileLoopType,
    ) -> Result<(Vec<Message>, Vec<i32>), MidiFileError> {
        let chunk_type = BinaryReader::read_four_cc(reader)?;
        if chunk_type != b"MTrk" {
            return Err(MidiFileError::InvalidChunkType {
                expected: FourCC::from_bytes(*b"MTrk"),
                actual: chunk_type,
            });
        }

        let size = BinaryReader::read_i32_big_endian(reader)? as usize;
        let reader = &mut ReadCounter::new(reader);

        let mut messages: Vec<Message> = Vec::new();
        let mut ticks: Vec<i32> = Vec::new();

        let mut tick: i32 = 0;
        let mut last_status: u8 = 0;

        // Bounded by the chunk size so that a track without an EOT meta event, or one truncated
        // mid-event, stops at its own chunk instead of parsing the next MTrk header as event data.
        while reader.bytes_read() < size {
            let delta = BinaryReader::read_i32_variable_length(reader)?;
            let first = BinaryReader::read_u8(reader)?;

            tick += delta;

            if (first & 128) == 0 {
                let command = last_status & 0xF0;
                if command == 0xC0 || command == 0xD0 {
                    messages.push(Message::common1(last_status, first));
                    ticks.push(tick);
                } else {
                    let data2 = BinaryReader::read_u8(reader)?;
                    messages.push(Message::common2(last_status, first, data2, loop_type));
                    ticks.push(tick);
                }

                continue;
            }

            match first {
                0xF0 => MidiFile::discard_data(reader)?,
                0xF7 => MidiFile::discard_data(reader)?,
                0xFF => match BinaryReader::read_u8(reader)? {
                    0x2F => {
                        BinaryReader::read_u8(reader)?;
                        messages.push(Message::EndOfTrack);
                        ticks.push(tick);

                        // Some MIDI files may have events inserted after the EOT.
                        // Such events should be ignored.
                        if reader.bytes_read() < size {
                            BinaryReader::discard_data(reader, size - reader.bytes_read())?;
                        }

                        return Ok((messages, ticks));
                    }
                    0x51 => {
                        messages.push(Message::tempo_change(MidiFile::read_tempo(reader)?));
                        ticks.push(tick);
                    }
                    _ => MidiFile::discard_data(reader)?,
                },

                // System common and system real-time. None of them means
                // anything to a synthesizer reading a file, but they have to be
                // consumed at their true length, because the arm below eats two
                // data bytes for anything it is handed. A single 0xF8 clock or
                // 0xFE active sensing byte left to fall through there swallows
                // the two bytes after it, and from that point every delta time,
                // status byte and key in the track is shifted - the whole rest
                // of the part comes out as wrong notes rather than as an error.
                // It also used to leave `last_status` at 0xF1..=0xFE, so every
                // following running-status event decoded as command 0xF0 and was
                // dropped by the synthesizer.
                0xF1 | 0xF3 => {
                    // MTC quarter frame, song select: one data byte.
                    BinaryReader::read_u8(reader)?;
                }
                0xF2 => {
                    // Song position pointer: two.
                    BinaryReader::read_u8(reader)?;
                    BinaryReader::read_u8(reader)?;
                }
                0xF4 | 0xF5 | 0xF6 | 0xF8..=0xFE => (),

                _ => {
                    let command = first & 0xF0;
                    if command == 0xC0 || command == 0xD0 {
                        let data1 = BinaryReader::read_u8(reader)?;
                        messages.push(Message::common1(first, data1));
                        ticks.push(tick);
                    } else {
                        let data1 = BinaryReader::read_u8(reader)?;
                        let data2 = BinaryReader::read_u8(reader)?;
                        messages.push(Message::common2(first, data1, data2, loop_type));
                        ticks.push(tick);
                    }

                    // Only a channel message sets the running status - this is
                    // the one arm that assigns it. The spec has SysEx and meta
                    // events cancel running status; this parser deliberately
                    // lets the previous channel status stand instead, because
                    // karaoke writers emit a lyric between a note-on and its
                    // running-status successor and every real player copes.
                    // Assigning here would put 0xF0 or 0xFF in `last_status`
                    // and silently drop that successor.
                    last_status = first;
                }
            }
        }

        // The chunk ended without an EOT meta event. Treat it as one rather than reading on into
        // the next track.
        messages.push(Message::EndOfTrack);
        ticks.push(tick);

        Ok((messages, ticks))
    }

    fn merge_tracks(
        message_lists: &[Vec<Message>],
        tick_lists: &[Vec<i32>],
        resolution: i32,
    ) -> (Vec<Message>, Vec<f64>) {
        let mut merged_messages: Vec<Message> = Vec::new();
        let mut merged_times: Vec<f64> = Vec::new();

        let mut indices: Vec<usize> = vec![0; message_lists.len()];

        let mut current_tick: i32 = 0;
        let mut current_time: f64 = 0.0;

        let mut tempo: f64 = 120.0;

        loop {
            let mut min_tick = i32::MAX;
            let mut min_index: i32 = -1;

            for ch in 0..tick_lists.len() {
                if indices[ch] < tick_lists[ch].len() {
                    let tick = tick_lists[ch][indices[ch]];
                    if tick < min_tick {
                        min_tick = tick;
                        min_index = ch as i32;
                    }
                }
            }

            if min_index == -1 {
                break;
            }

            let next_tick = tick_lists[min_index as usize][indices[min_index as usize]];
            let delta_tick = next_tick - current_tick;
            let delta_time = 60.0 / (resolution as f64 * tempo) * delta_tick as f64;

            current_tick += delta_tick;
            current_time += delta_time;

            let message = message_lists[min_index as usize][indices[min_index as usize]];
            if let Message::TempoChange { bytes } = message {
                let tempo_i32 = i32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]);
                tempo = 60000000.0 / tempo_i32 as f64;
            } else {
                merged_messages.push(message);
                merged_times.push(current_time);
            }

            indices[min_index as usize] += 1;
        }

        (merged_messages, merged_times)
    }

    /// Get the length of the MIDI file in seconds.
    pub fn get_length(&self) -> f64 {
        *self.times.last().unwrap()
    }

    /// Gets the channel messages of the MIDI file, in time order, with the
    /// tempo map already resolved to seconds.
    ///
    /// This is the same sequence `MidiFileSequencer` plays, so a host that
    /// wants to drive `Synthesizer::process_midi_message` itself - to silence a
    /// channel, to render one part on its own, or to report what a file asked
    /// for - gets the same events the sequencer would have delivered.
    ///
    /// Only channel messages are reported. Tempo changes are already folded
    /// into each event's time, and the loop markers and end-of-track records
    /// are not channel messages.
    pub fn get_events(&self) -> impl Iterator<Item = MidiEvent> + '_ {
        self.messages
            .iter()
            .zip(self.times.iter())
            .filter_map(|(message, time)| match message {
                Message::Normal {
                    status,
                    data1,
                    data2,
                } => Some(MidiEvent {
                    time: *time,
                    channel: status & 0x0F,
                    command: status & 0xF0,
                    data1: *data1,
                    data2: *data2,
                }),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    #[test]
    fn test_message_size() {
        // Avoid increasing the size of the Message type
        assert_eq!(size_of::<Message>(), 4);
    }

    fn header(format: u16, track_count: u16) -> Vec<u8> {
        let mut data = b"MThd   ".to_vec();
        data.extend_from_slice(&format.to_be_bytes());
        data.extend_from_slice(&track_count.to_be_bytes());
        data.extend_from_slice(&96u16.to_be_bytes());
        data
    }

    fn track(events: &[u8]) -> Vec<u8> {
        let mut data = b"MTrk".to_vec();
        data.extend_from_slice(&(events.len() as u32).to_be_bytes());
        data.extend_from_slice(events);
        data
    }

    #[test]
    fn test_meta_event_cancels_running_status() {
        // A meta event terminates running status, so the note-on that follows it in running status
        // still belongs to the 0x90 that came before, not to the meta event's 0xFF.
        let mut data = header(0, 1);
        data.extend_from_slice(&track(&[
            0x00, 0x90, 0x3C, 0x64, // note on, key 60 - sets the running status
            0x00, 0xFF, 0x01, 0x01, 0x41, // text meta event "A"
            0x00, 0x3E, 0x64, // note on, key 62, in running status
            0x00, 0xFF, 0x2F, 0x00, // end of track
        ]));

        let midi_file = MidiFile::new(&mut Cursor::new(data)).unwrap();

        let notes: Vec<(u8, u8, u8)> = midi_file
            .messages
            .iter()
            .filter_map(|message| match *message {
                Message::Normal {
                    status,
                    data1,
                    data2,
                } => Some((status, data1, data2)),
                _ => None,
            })
            .collect();

        assert_eq!(notes, vec![(0x90, 60, 100), (0x90, 62, 100)]);
    }

    #[test]
    fn test_track_without_end_of_track_stops_at_the_chunk_end() {
        // The first track has no EOT meta event. It must stop at its own chunk rather than reading
        // the next MTrk header as event data.
        let mut data = header(1, 2);
        data.extend_from_slice(&track(&[
            0x00, 0x90, 0x3C, 0x64, // note on, key 60 - and then nothing
        ]));
        data.extend_from_slice(&track(&[
            0x00, 0x90, 0x3E, 0x64, // note on, key 62
            0x00, 0xFF, 0x2F, 0x00, // end of track
        ]));

        let midi_file = MidiFile::new(&mut Cursor::new(data)).unwrap();

        let notes: Vec<(u8, u8, u8)> = midi_file
            .messages
            .iter()
            .filter_map(|message| match *message {
                Message::Normal {
                    status,
                    data1,
                    data2,
                } => Some((status, data1, data2)),
                _ => None,
            })
            .collect();

        assert_eq!(notes, vec![(0x90, 60, 100), (0x90, 62, 100)]);
    }

    #[test]
    fn test_realtime_status_bytes_do_not_desynchronise_the_track() {
        // A clock, an active sensing and a tune request byte carry no data. Read
        // as two-byte channel messages they eat the events after them and every
        // following note in the track comes out at the wrong pitch.
        let mut data = header(0, 1);
        data.extend_from_slice(&track(&[
            0x00, 0xF8, // clock - no data bytes
            0x00, 0x90, 0x3C, 0x64, // note on, key 60
            0x00, 0xFE, // active sensing - no data bytes
            0x00, 0x3E, 0x64, // note on, key 62, in running status
            0x00, 0xF6, // tune request - no data bytes
            0x00, 0xF1, 0x21, // MTC quarter frame - one data byte
            0x00, 0xF3, 0x05, // song select - one data byte
            0x00, 0xF2, 0x00, 0x10, // song position pointer - two data bytes
            0x00, 0x40, 0x64, // note on, key 64, still in running status
            0x00, 0xFF, 0x2F, 0x00, // end of track
        ]));

        let midi_file = MidiFile::new(&mut Cursor::new(data)).unwrap();

        let events: Vec<(i32, i32, i32)> = midi_file
            .get_events()
            .map(|event| (event.get_command(), event.get_data1(), event.get_data2()))
            .collect();

        assert_eq!(
            events,
            vec![(0x90, 60, 100), (0x90, 62, 100), (0x90, 64, 100)]
        );
    }

    #[test]
    fn test_get_events_splits_the_status_byte_and_resolves_the_tempo() {
        // get_events has to hand out what the sequencer would have dispatched: the status byte
        // already split into channel and command, the tempo map already resolved to seconds, and
        // nothing that is not a channel message.
        let mut data = header(0, 1);
        data.extend_from_slice(&track(&[
            // 240000 us per quarter note, so a quarter note is 0.24 s and one tick is 0.0025 s.
            0x00, 0xFF, 0x51, 0x03, 0x03, 0xA9, 0x80, //
            0x00, 0x94, 0x3C, 0x64, // note on, channel 4, key 60
            0x60, 0xB2, 0x07, 0x40, // 96 ticks later, CC7 on channel 2
            0x00, 0xC5, 0x19, // program change on channel 5 - one data byte
            0x00, 0xFF, 0x2F, 0x00, // end of track
        ]));

        let midi_file = MidiFile::new(&mut Cursor::new(data)).unwrap();

        let events: Vec<(f64, i32, i32, i32, i32)> = midi_file
            .get_events()
            .map(|event| {
                (
                    event.get_time(),
                    event.get_channel(),
                    event.get_command(),
                    event.get_data1(),
                    event.get_data2(),
                )
            })
            .collect();

        assert_eq!(
            events,
            vec![
                (0.0, 4, 0x90, 60, 100),
                (0.24, 2, 0xB0, 7, 64),
                (0.24, 5, 0xC0, 25, 0),
            ]
        );
    }
}
