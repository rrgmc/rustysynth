#![allow(dead_code)]

#[derive(Debug, PartialEq, Eq)]
enum DataType {
    None,
    Rpn,
    Nrpn,
}

#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct Channel {
    pub(crate) is_percussion_channel: bool,

    bank_number: i32,
    patch_number: i32,

    modulation: i16,
    volume: i16,
    pan: i16,
    expression: i16,
    hold_pedal: bool,

    /// Raw 7-bit value of every controller, so that modulators can name one
    /// this synthesizer has no dedicated field for. CC1, CC7, CC10 and CC11
    /// are also tracked at 14 bits above and are read from there instead;
    /// their entries here are kept consistent but unused.
    cc: [u8; 128],
    channel_pressure: u8,
    poly_pressure: [u8; 128],

    rpn: i16,
    pitch_bend_range: i16,
    coarse_tune: i16,
    fine_tune: i16,

    pitch_bend: f32,

    last_data_type: DataType,
}

impl Channel {
    const CC_MODULATION: usize = 1;
    const CC_VOLUME: usize = 7;
    const CC_PAN: usize = 10;
    const CC_EXPRESSION: usize = 11;
    const CC_HOLD_PEDAL: usize = 64;
    const CC_REVERB_SEND: usize = 91;
    const CC_CHORUS_SEND: usize = 93;

    pub(crate) fn new(is_percussion_channel: bool) -> Self {
        let mut channel = Self {
            is_percussion_channel,
            bank_number: 0,
            patch_number: 0,
            modulation: 0,
            volume: 0,
            pan: 0,
            expression: 0,
            hold_pedal: false,
            cc: [0; 128],
            channel_pressure: 0,
            poly_pressure: [0; 128],
            rpn: 0,
            pitch_bend_range: 0,
            coarse_tune: 0,
            fine_tune: 0,
            pitch_bend: 0_f32,
            last_data_type: DataType::None,
        };

        channel.reset();

        channel
    }

    pub(crate) fn reset(&mut self) {
        self.bank_number = if self.is_percussion_channel { 128 } else { 0 };
        self.patch_number = 0;

        self.modulation = 0;
        self.volume = 100 << 7;
        self.pan = 64 << 7;
        self.expression = 127 << 7;
        self.hold_pedal = false;

        self.cc = [0; 128];
        // Keep the raw entries for the 14-bit controllers consistent with the
        // fields above, and preserve the long-standing default of a nonzero
        // reverb send.
        self.cc[Channel::CC_MODULATION] = 0;
        self.cc[Channel::CC_VOLUME] = 100;
        self.cc[Channel::CC_PAN] = 64;
        self.cc[Channel::CC_EXPRESSION] = 127;
        self.cc[Channel::CC_REVERB_SEND] = 40;
        self.cc[Channel::CC_CHORUS_SEND] = 0;

        self.channel_pressure = 0;
        self.poly_pressure = [0; 128];

        self.rpn = -1;
        self.pitch_bend_range = 2 << 7;
        self.coarse_tune = 0;
        self.fine_tune = 8192;

        self.pitch_bend = 0_f32;
    }

    pub(crate) fn reset_all_controllers(&mut self) {
        self.modulation = 0;
        self.expression = 127 << 7;
        self.hold_pedal = false;

        self.rpn = -1;

        self.pitch_bend = 0_f32;

        // Mirror the above into the raw controller state, and reset the
        // pressures with it. Volume, pan and the effect sends are deliberately
        // left alone, matching both the GM spec and this method's existing
        // behavior.
        self.cc[Channel::CC_MODULATION] = 0;
        self.cc[Channel::CC_EXPRESSION] = 127;
        self.cc[Channel::CC_HOLD_PEDAL] = 0;

        self.channel_pressure = 0;
        self.poly_pressure = [0; 128];
    }

    // MIDI data bytes are seven bits by definition, but a malformed file can
    // still deliver a larger one - the corpus has files where a byte lands in
    // CC11 as 255. Left unmasked that made `expression` 1.99 rather than at
    // most 1, and the old `(volume * expression)^2` channel gain turned it into
    // a 2.4x boost that clipped. Masking here keeps every controller inside
    // the range its getters promise.
    pub(crate) fn set_bank(&mut self, value: i32) {
        self.bank_number = value;

        if self.is_percussion_channel {
            self.bank_number += 128;
        }
    }

    pub(crate) fn set_patch(&mut self, value: i32) {
        self.patch_number = value;
    }

    pub(crate) fn set_modulation_coarse(&mut self, value: i32) {
        self.modulation = (self.modulation & 0x7F) | ((value & 0x7F) << 7) as i16;
    }

    pub(crate) fn set_modulation_fine(&mut self, value: i32) {
        self.modulation = (((self.modulation as i32) & 0xFF80) | (value & 0x7F)) as i16;
    }

    pub(crate) fn set_volume_coarse(&mut self, value: i32) {
        self.volume = (self.volume & 0x7F) | ((value & 0x7F) << 7) as i16;
    }

    pub(crate) fn set_volume_fine(&mut self, value: i32) {
        self.volume = (((self.volume as i32) & 0xFF80) | (value & 0x7F)) as i16;
    }

    pub(crate) fn set_pan_coarse(&mut self, value: i32) {
        self.pan = (self.pan & 0x7F) | ((value & 0x7F) << 7) as i16;
    }

    pub(crate) fn set_pan_fine(&mut self, value: i32) {
        self.pan = (((self.pan as i32) & 0xFF80) | (value & 0x7F)) as i16;
    }

    pub(crate) fn set_expression_coarse(&mut self, value: i32) {
        self.expression = (self.expression & 0x7F) | ((value & 0x7F) << 7) as i16;
    }

    pub(crate) fn set_expression_fine(&mut self, value: i32) {
        self.expression = (((self.expression as i32) & 0xFF80) | (value & 0x7F)) as i16;
    }

    pub(crate) fn set_hold_pedal(&mut self, value: i32) {
        self.hold_pedal = value >= 64;
    }

    pub(crate) fn set_reverb_send(&mut self, value: i32) {
        self.cc[Channel::CC_REVERB_SEND] = (value & 0x7F) as u8;
    }

    pub(crate) fn set_chorus_send(&mut self, value: i32) {
        self.cc[Channel::CC_CHORUS_SEND] = (value & 0x7F) as u8;
    }

    /// Records a controller this synthesizer has no dedicated field for, so
    /// that a font modulator naming it can be honored.
    pub(crate) fn set_cc(&mut self, index: i32, value: i32) {
        if (0..128).contains(&index) {
            self.cc[index as usize] = (value & 0x7F) as u8;
        }
    }

    pub(crate) fn set_channel_pressure(&mut self, value: i32) {
        self.channel_pressure = (value & 0x7F) as u8;
    }

    pub(crate) fn set_poly_pressure(&mut self, key: i32, value: i32) {
        if (0..128).contains(&key) {
            self.poly_pressure[key as usize] = (value & 0x7F) as u8;
        }
    }

    pub(crate) fn set_rpn_coarse(&mut self, value: i32) {
        self.rpn = (self.rpn & 0x7F) | (value << 7) as i16;
        self.last_data_type = DataType::Rpn;
    }

    pub(crate) fn set_rpn_fine(&mut self, value: i32) {
        self.rpn = (((self.rpn as i32) & 0xFF80) | value) as i16;
        self.last_data_type = DataType::Rpn;
    }

    pub(crate) fn set_nrpn_coarse(&mut self, _value: i32) {
        self.last_data_type = DataType::Nrpn;
    }

    pub(crate) fn set_nrpn_fine(&mut self, _value: i32) {
        self.last_data_type = DataType::Nrpn;
    }

    pub(crate) fn data_entry_coarse(&mut self, value: i32) {
        if self.last_data_type != DataType::Rpn {
            return;
        }

        if self.rpn == 0 {
            self.pitch_bend_range = (self.pitch_bend_range & 0x7F) | (value << 7) as i16;
        } else if self.rpn == 1 {
            self.fine_tune = (self.fine_tune & 0x7F) | (value << 7) as i16;
        } else if self.rpn == 2 {
            self.coarse_tune = (value - 64) as i16;
        }
    }

    pub(crate) fn data_entry_fine(&mut self, value: i32) {
        if self.last_data_type != DataType::Rpn {
            return;
        }

        if self.rpn == 0 {
            self.pitch_bend_range = (((self.pitch_bend_range as i32) & 0xFF80) | value) as i16;
        } else if self.rpn == 1 {
            self.fine_tune = (((self.fine_tune as i32) & 0xFF80) | value) as i16;
        }
    }

    pub(crate) fn set_pitch_bend(&mut self, value1: i32, value2: i32) {
        self.pitch_bend = (1_f32 / 8192_f32) * ((value1 | (value2 << 7)) - 8192) as f32;
    }

    pub(crate) fn get_bank_number(&self) -> i32 {
        self.bank_number
    }

    pub(crate) fn get_patch_number(&self) -> i32 {
        self.patch_number
    }

    pub(crate) fn get_modulation(&self) -> f32 {
        (50_f32 / 16383_f32) * self.modulation as f32
    }

    pub(crate) fn get_volume(&self) -> f32 {
        (1_f32 / 16383_f32) * self.volume as f32
    }

    pub(crate) fn get_pan(&self) -> f32 {
        (100_f32 / 16383_f32) * self.pan as f32 - 50_f32
    }

    pub(crate) fn get_expression(&self) -> f32 {
        (1_f32 / 16383_f32) * self.expression as f32
    }

    pub(crate) fn get_hold_pedal(&self) -> bool {
        self.hold_pedal
    }

    pub(crate) fn get_reverb_send(&self) -> f32 {
        (1_f32 / 127_f32) * self.cc[Channel::CC_REVERB_SEND] as f32
    }

    pub(crate) fn get_chorus_send(&self) -> f32 {
        (1_f32 / 127_f32) * self.cc[Channel::CC_CHORUS_SEND] as f32
    }

    pub(crate) fn get_cc(&self, index: u8) -> u8 {
        self.cc[(index & 0x7F) as usize]
    }

    pub(crate) fn get_channel_pressure(&self) -> u8 {
        self.channel_pressure
    }

    pub(crate) fn get_poly_pressure(&self, key: i32) -> u8 {
        if (0..128).contains(&key) {
            self.poly_pressure[key as usize]
        } else {
            0
        }
    }

    /// Modulation depth as a plain 0..1 fraction, for modulator sources.
    /// `get_modulation` returns cents instead, for the legacy vibrato path.
    pub(crate) fn get_modulation_normalized(&self) -> f32 {
        (1_f32 / 16383_f32) * self.modulation as f32
    }

    /// Pan as a 0..1 fraction rather than the -50..50 that `get_pan` returns.
    pub(crate) fn get_pan_normalized(&self) -> f32 {
        (1_f32 / 16383_f32) * self.pan as f32
    }

    /// Pitch wheel deflection as a 0..1 fraction, centred at 0.5, which is
    /// what a modulator source expects. `get_pitch_bend` returns semitones.
    pub(crate) fn get_pitch_bend_normalized(&self) -> f32 {
        0.5_f32 * (self.pitch_bend + 1_f32)
    }

    pub(crate) fn get_pitch_bend_range(&self) -> f32 {
        (self.pitch_bend_range >> 7) as f32 + 0.01_f32 * (self.pitch_bend_range & 0x7F) as f32
    }

    pub(crate) fn get_tune(&self) -> f32 {
        self.coarse_tune as f32 + (1_f32 / 8192_f32) * (self.fine_tune - 8192) as f32
    }

    pub(crate) fn get_pitch_bend(&self) -> f32 {
        self.get_pitch_bend_range() * self.pitch_bend
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The effect sends are stored in the raw controller array now rather than
    /// in fields of their own, so the long-standing quirk that a reset leaves
    /// the reverb send at 40 has to survive the move.
    #[test]
    fn reset_leaves_the_reverb_send_at_forty() {
        let mut channel = Channel::new(false);

        channel.set_reverb_send(0);
        assert_eq!(channel.get_reverb_send(), 0_f32);

        channel.reset();
        assert_eq!(channel.get_cc(91), 40);
        assert!((channel.get_reverb_send() - 40_f32 / 127_f32).abs() < 1.0e-6);
        assert_eq!(channel.get_chorus_send(), 0_f32);
    }

    /// Reset all controllers deliberately preserves volume, pan and the sends.
    /// A modulator reading any of them has to see the same value the dedicated
    /// getter does.
    #[test]
    fn reset_all_controllers_preserves_volume_pan_and_sends() {
        let mut channel = Channel::new(false);

        channel.set_volume_coarse(90);
        channel.set_pan_coarse(20);
        channel.set_reverb_send(100);
        channel.set_chorus_send(80);
        channel.set_cc(91, 100);
        channel.set_cc(93, 80);
        channel.set_modulation_coarse(64);
        channel.set_cc(1, 64);
        channel.set_channel_pressure(90);
        channel.set_poly_pressure(60, 70);

        channel.reset_all_controllers();

        assert_eq!(channel.get_cc(91), 100);
        assert_eq!(channel.get_cc(93), 80);
        assert!((channel.get_volume() - (90 << 7) as f32 / 16383_f32).abs() < 1.0e-6);
        assert!(
            (channel.get_pan() - ((100_f32 / 16383_f32) * (20 << 7) as f32 - 50_f32)).abs()
                < 1.0e-4
        );

        // Modulation, expression and both pressures are reset.
        assert_eq!(channel.get_modulation(), 0_f32);
        assert_eq!(channel.get_cc(1), 0);
        assert_eq!(channel.get_cc(11), 127);
        assert_eq!(channel.get_channel_pressure(), 0);
        assert_eq!(channel.get_poly_pressure(60), 0);
    }

    /// A modulator may name any controller, including ones out of range and
    /// ones this synthesizer has no dedicated handling for.
    #[test]
    fn arbitrary_controllers_are_recorded_and_read_back() {
        let mut channel = Channel::new(false);

        channel.set_cc(74, 100);
        assert_eq!(channel.get_cc(74), 100);

        // Values are masked to seven bits, and out-of-range indices are
        // ignored rather than panicking.
        channel.set_cc(74, 200);
        assert_eq!(channel.get_cc(74), 200 & 0x7F);
        channel.set_cc(-1, 100);
        channel.set_cc(999, 100);

        channel.set_poly_pressure(-5, 100);
        channel.set_poly_pressure(999, 100);
        assert_eq!(channel.get_poly_pressure(-5), 0);
        assert_eq!(channel.get_poly_pressure(999), 0);
    }

    /// Pitch wheel sensitivity is read through the fractional accessor, not by
    /// shifting the raw value, so an RPN 0 fine adjustment is not silently
    /// dropped.
    #[test]
    fn pitch_bend_normalization_spans_zero_to_one() {
        let mut channel = Channel::new(false);

        channel.set_pitch_bend(0, 0);
        assert!(channel.get_pitch_bend_normalized().abs() < 1.0e-4);

        channel.set_pitch_bend(0, 64);
        assert!((channel.get_pitch_bend_normalized() - 0.5_f32).abs() < 1.0e-4);

        channel.set_pitch_bend(0x7F, 0x7F);
        assert!((channel.get_pitch_bend_normalized() - 1_f32).abs() < 1.0e-3);
    }
    /// A malformed MIDI file can deliver a data byte with the high bit set.
    /// The corpus has files that land 255 in CC11, and unmasked that made
    /// `expression` 1.99 rather than at most 1 - which the old squared channel
    /// gain turned into a 2.4x boost that clipped.
    #[test]
    fn out_of_range_controller_values_cannot_exceed_full_scale() {
        let mut channel = Channel::new(false);

        channel.set_expression_coarse(255);
        assert!(channel.get_expression() <= 1_f32);
        assert!((channel.get_expression() - (127 << 7) as f32 / 16383_f32).abs() < 1.0e-6);

        channel.set_volume_coarse(255);
        assert!(channel.get_volume() <= 1_f32);

        channel.set_modulation_coarse(255);
        assert!(channel.get_modulation() <= 50_f32);

        channel.set_pan_coarse(255);
        assert!(channel.get_pan() <= 50_f32);

        channel.set_expression_fine(255);
        channel.set_volume_fine(255);
        assert!(channel.get_expression() <= 1_f32);
        assert!(channel.get_volume() <= 1_f32);

        channel.set_reverb_send(255);
        channel.set_chorus_send(255);
        assert!(channel.get_reverb_send() <= 1_f32);
        assert!(channel.get_chorus_send() <= 1_f32);
    }
}
