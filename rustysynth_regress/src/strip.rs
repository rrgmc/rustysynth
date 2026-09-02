//! Produces a modulator-free copy of a SoundFont.
//!
//! None of the fonts available for testing ships without modulators, so the
//! "does this still render what it used to?" comparison has no control font
//! unless one is manufactured. Emptying `pmod` and `imod` leaves a font whose
//! synthesis is driven entirely by the SF2 default modulators, which is exactly
//! the case that has to reproduce the old hardcoded controller handling.

use std::fs;
use std::path::Path;

/// A modulator list holding nothing but its terminator record.
const EMPTY_MODULATOR_LIST: [u8; 10] = [0; 10];

/// Points every zone at the start of the now-empty modulator list.
///
/// Emptying `pmod` and `imod` alone is not enough: each `pbag` or `ibag` record
/// is a pair of u16 indices, and the modulator one would still point into a
/// list that no longer has those entries. The library rejects that, correctly.
fn clear_bag_modulator_indices(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for record in out.as_chunks_mut::<4>().0 {
        record[2] = 0;
        record[3] = 0;
    }
    out
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("truncated chunk header at {at}"))
}

fn chunk_id(bytes: &[u8], at: usize) -> Result<[u8; 4], String> {
    bytes
        .get(at..at + 4)
        .map(|b| [b[0], b[1], b[2], b[3]])
        .ok_or_else(|| format!("truncated chunk id at {at}"))
}

/// Rewrites the sub-chunks of a `pdta` list, emptying the two modulator lists.
fn rewrite_pdta(body: &[u8]) -> Result<Vec<u8>, String> {
    // The list type, "pdta", stays as it is.
    let mut out: Vec<u8> = body[0..4].to_vec();

    let mut at = 4_usize;
    while at + 8 <= body.len() {
        let id = chunk_id(body, at)?;
        let size = read_u32(body, at + 4)? as usize;
        let data_start = at + 8;
        let data_end = data_start
            .checked_add(size)
            .filter(|end| *end <= body.len())
            .ok_or_else(|| {
                format!(
                    "chunk {} overruns the pdta list",
                    String::from_utf8_lossy(&id)
                )
            })?;

        let cleared_bag;
        let replacement: &[u8] = if &id == b"pmod" || &id == b"imod" {
            &EMPTY_MODULATOR_LIST
        } else if &id == b"pbag" || &id == b"ibag" {
            cleared_bag = clear_bag_modulator_indices(&body[data_start..data_end]);
            &cleared_bag
        } else {
            &body[data_start..data_end]
        };

        out.extend_from_slice(&id);
        out.extend_from_slice(&(replacement.len() as u32).to_le_bytes());
        out.extend_from_slice(replacement);
        if !replacement.len().is_multiple_of(2) {
            out.push(0);
        }

        at = data_end + usize::from(!size.is_multiple_of(2));
    }

    Ok(out)
}

pub fn run(input: &Path, output: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;

    if chunk_id(&bytes, 0)? != *b"RIFF" || chunk_id(&bytes, 8)? != *b"sfbk" {
        return Err(format!("{} is not a SoundFont", input.display()));
    }

    let riff_size = read_u32(&bytes, 4)? as usize;
    let riff_end = (8 + riff_size).min(bytes.len());

    // Everything after "RIFF<size>" - the form type and every top-level chunk.
    let mut body: Vec<u8> = bytes[8..12].to_vec();

    let mut at = 12_usize;
    while at + 8 <= riff_end {
        let id = chunk_id(&bytes, at)?;
        let size = read_u32(&bytes, at + 4)? as usize;
        let data_start = at + 8;
        let data_end = data_start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| format!("chunk {} overruns the file", String::from_utf8_lossy(&id)))?;

        let data = &bytes[data_start..data_end];
        let is_pdta = &id == b"LIST" && data.len() >= 4 && &data[0..4] == b"pdta";

        let replacement = if is_pdta {
            rewrite_pdta(data)?
        } else {
            data.to_vec()
        };

        body.extend_from_slice(&id);
        body.extend_from_slice(&(replacement.len() as u32).to_le_bytes());
        body.extend_from_slice(&replacement);
        if !replacement.len().is_multiple_of(2) {
            body.push(0);
        }

        at = data_end + usize::from(!size.is_multiple_of(2));
    }

    let mut out: Vec<u8> = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);

    fs::write(output, &out).map_err(|e| format!("{}: {e}", output.display()))?;

    println!(
        "{} -> {} ({} bytes -> {} bytes)",
        input.display(),
        output.display(),
        bytes.len(),
        out.len()
    );

    Ok(())
}
