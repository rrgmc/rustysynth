#![allow(dead_code)]

use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::str;

use crate::four_cc::FourCC;

#[allow(unused)]
#[non_exhaustive]
pub(crate) struct BinaryReader {}

impl BinaryReader {
    pub(crate) fn read_i8<R: Read>(reader: &mut R) -> Result<i8, io::Error> {
        let mut data: [u8; 1] = [0; 1];
        reader.read_exact(&mut data)?;
        Ok(i8::from_le_bytes(data))
    }

    pub(crate) fn read_u8<R: Read>(reader: &mut R) -> Result<u8, io::Error> {
        let mut data: [u8; 1] = [0; 1];
        reader.read_exact(&mut data)?;
        Ok(u8::from_le_bytes(data))
    }

    pub(crate) fn read_i16<R: Read>(reader: &mut R) -> Result<i16, io::Error> {
        let mut data: [u8; 2] = [0; 2];
        reader.read_exact(&mut data)?;
        Ok(i16::from_le_bytes(data))
    }

    pub(crate) fn read_u16<R: Read>(reader: &mut R) -> Result<u16, io::Error> {
        let mut data: [u8; 2] = [0; 2];
        reader.read_exact(&mut data)?;
        Ok(u16::from_le_bytes(data))
    }

    pub(crate) fn read_i32<R: Read>(reader: &mut R) -> Result<i32, io::Error> {
        let mut data: [u8; 4] = [0; 4];
        reader.read_exact(&mut data)?;
        Ok(i32::from_le_bytes(data))
    }

    pub(crate) fn read_u32<R: Read>(reader: &mut R) -> Result<u32, io::Error> {
        let mut data: [u8; 4] = [0; 4];
        reader.read_exact(&mut data)?;
        Ok(u32::from_le_bytes(data))
    }

    pub(crate) fn read_i16_big_endian<R: Read>(reader: &mut R) -> Result<i16, io::Error> {
        let mut data: [u8; 2] = [0; 2];
        reader.read_exact(&mut data)?;
        Ok(i16::from_be_bytes(data))
    }

    pub(crate) fn read_i32_big_endian<R: Read>(reader: &mut R) -> Result<i32, io::Error> {
        let mut data: [u8; 4] = [0; 4];
        reader.read_exact(&mut data)?;
        Ok(i32::from_be_bytes(data))
    }

    pub(crate) fn read_i32_variable_length<R: Read>(reader: &mut R) -> Result<i32, io::Error> {
        let mut acc: i32 = 0;
        let mut count: i32 = 0;

        loop {
            let value = BinaryReader::read_u8(reader)? as i32;
            acc = (acc << 7) | (value & 127);
            if (value & 128) == 0 {
                break;
            }
            count += 1;
            if count == 4 {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "the length of the value must be equal to or less than 4",
                ));
            }
        }

        Ok(acc)
    }

    pub(crate) fn read_four_cc<R: Read>(reader: &mut R) -> Result<FourCC, io::Error> {
        let mut data: [u8; 4] = [0; 4];
        reader.read_exact(&mut data)?;
        Ok(FourCC::from_bytes(data))
    }

    pub(crate) fn read_fixed_length_string<R: Read>(
        reader: &mut R,
        length: usize,
    ) -> Result<String, io::Error> {
        let mut data: Vec<u8> = vec![0; length];
        reader.read_exact(&mut data)?;

        let mut actual_length: usize = 0;
        for value in &mut data {
            if *value == 0 {
                break;
            }
            actual_length += 1;
        }

        // Replace non-ASCII characters with '?'.
        // Tabs and returns are preserved.
        for value in &mut data[0..actual_length] {
            if !(9..=126).contains(value) {
                *value = 63; // '?'
            }
        }

        Ok(str::from_utf8(&data[0..actual_length]).unwrap().to_string())
    }

    /// Reads and throws away `size` bytes.
    ///
    /// Deliberately does not allocate `size`: the value comes straight out of
    /// the file, and a chunk header claiming 4 GB should cost a long read
    /// ending in `UnexpectedEof`, not a 4 GB allocation before the first byte
    /// is read.
    pub(crate) fn discard_data<R: Read>(reader: &mut R, size: usize) -> Result<(), io::Error> {
        let discarded = io::copy(&mut reader.take(size as u64), &mut io::sink())?;
        if discarded != size as u64 {
            return Err(io::Error::from(ErrorKind::UnexpectedEof));
        }

        Ok(())
    }

    /// Reads the `smpl` chunk into 16-bit samples.
    ///
    /// Read in blocks rather than straight into a byte view of the `Vec<i16>`.
    /// The view used to be built at `size` bytes over an allocation of
    /// `size / 2` samples, so an odd `size` - which nothing rejects, and which
    /// the file controls - wrote one byte past the end of the allocation.
    /// Reading blockwise is also correct on a big-endian target, which the raw
    /// pointer read was not.
    ///
    /// The capacity is still reserved up front, because this is where a
    /// gigabyte font spends its memory and growing into it would need a second
    /// copy. `try_reserve_exact` rather than `with_capacity` so that a chunk
    /// header claiming more than there is to give is an error rather than an
    /// abort; nothing is written into the reservation until the bytes actually
    /// arrive, so an impossible `size` costs address space and not pages.
    pub(crate) fn read_wave_data<R: Read>(
        reader: &mut R,
        size: usize,
    ) -> Result<Vec<i16>, io::Error> {
        const BLOCK_BYTES: usize = 1 << 16;

        let length = size / 2;
        let mut samples: Vec<i16> = Vec::new();
        samples
            .try_reserve_exact(length)
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "the sample data is too large"))?;
        let mut block = vec![0_u8; BLOCK_BYTES];

        let mut remaining = length;
        while remaining > 0 {
            let wanted = 2 * remaining.min(BLOCK_BYTES / 2);
            let block = &mut block[..wanted];
            reader.read_exact(block)?;
            samples.extend(
                block
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| i16::from_le_bytes(*pair)),
            );
            remaining -= wanted / 2;
        }

        // An odd chunk size leaves a trailing byte that is not part of any
        // sample. It still has to come off the stream, or everything after it
        // reads misaligned.
        if size % 2 == 1 {
            BinaryReader::discard_data(reader, 1)?;
        }

        Ok(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An odd `smpl` size used to overflow the heap allocation by one byte.
    #[test]
    fn an_odd_wave_data_size_is_read_without_overflowing() {
        let data: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let samples = BinaryReader::read_wave_data(&mut data.as_slice(), data.len()).unwrap();

        assert_eq!(samples, vec![0x0201, 0x0403]);
    }

    #[test]
    fn wave_data_is_read_little_endian_across_block_boundaries() {
        let length = 100_000;
        let mut data: Vec<u8> = Vec::with_capacity(2 * length);
        for i in 0..length {
            data.extend_from_slice(&(i as i16).to_le_bytes());
        }

        let samples = BinaryReader::read_wave_data(&mut data.as_slice(), data.len()).unwrap();

        assert_eq!(samples.len(), length);
        assert_eq!(samples[0], 0);
        assert_eq!(samples[40_000], 40_000_i32 as i16);
        assert_eq!(samples[length - 1], (length - 1) as i16);
    }

    /// An impossible chunk size has to fail rather than abort the process on a
    /// failed allocation.
    #[test]
    fn an_impossible_wave_chunk_size_is_an_error() {
        let data: Vec<u8> = vec![0x01, 0x02];
        assert!(BinaryReader::read_wave_data(&mut data.as_slice(), usize::MAX / 2).is_err());
    }

    #[test]
    fn a_truncated_wave_chunk_is_an_error() {
        let data: Vec<u8> = vec![0x01, 0x02];
        assert!(BinaryReader::read_wave_data(&mut data.as_slice(), 64).is_err());
    }

    /// `discard_data` used to allocate what the chunk header claimed before
    /// reading a byte of it.
    #[test]
    fn discarding_more_than_is_there_is_an_error() {
        let data: Vec<u8> = vec![0x01, 0x02];
        assert!(BinaryReader::discard_data(&mut data.as_slice(), 0xFFFF_FFFF).is_err());
        assert!(BinaryReader::discard_data(&mut data.as_slice(), 2).is_ok());
    }
}
