"""Builds the small malformed SF2 fixtures the lenient-loading tests use.

Each one carries exactly one defect, so that a test asserting on it is asserting
on that defect and not on whatever else a real broken bank happens to contain.
They stand in for banks far too large to commit: `test_bad_region` is the shape
of Crisis General Midi 3.01, whose one bad sample header out of 5,007 used to
cost the whole 1,611 MiB file, and `test_empty_instrument` is the shape of
Timbres of Heaven 4.00, whose instrument 9 has an empty bag span.

Run from the workspace root:

    python samples/make_test_malformed.py
"""
import io
import struct

NUL = bytes([0])

# Generator numbers used below.
INITIAL_ATTENUATION = 48
INSTRUMENT = 41
SAMPLE_ID = 53

# A shared pool of silence that every fixture's sample headers address into.
WAVE_SAMPLES = 256
WAVE_DATA = bytes(2 * WAVE_SAMPLES)


def chunk(fourcc, data):
    """A RIFF chunk, padded to an even length as RIFF requires."""
    padded = data + NUL if len(data) % 2 else data
    return fourcc + struct.pack('<I', len(data)) + padded


def name20(text):
    return text.encode('ascii')[:20].ljust(20, NUL)


def generator(kind, value):
    return struct.pack('<HH', kind, value & 0xFFFF)


def shdr_record(name, start, end, start_loop, end_loop, pitch=60):
    return (name20(name)
            + struct.pack('<IIIII', start, end, start_loop, end_loop, 44100)
            + struct.pack('<BbHH', pitch, 0, 0, 1))


TERMINATOR_GENERATOR = generator(0, 0)
TERMINATOR_MODULATOR = struct.pack('<HHhHH', 0, 0, 0, 0, 0)


def bag_and_records(zone_lists):
    """Flattens zones into a bag table and its generator and modulator chunks.

    `zone_lists` is one list of zones per instrument or preset, each zone being
    a list of (generator, value) pairs. The bag table gets one entry per zone
    plus the terminator that bounds the last one, exactly as the SF2 layout
    derives every count from the next record's index.
    """
    bag = b''
    gens = b''
    generator_index = 0

    for zones in zone_lists:
        for zone in zones:
            bag += struct.pack('<HH', generator_index, 0)
            for kind, value in zone:
                gens += generator(kind, value)
                generator_index += 1

    bag += struct.pack('<HH', generator_index, 0)
    gens += TERMINATOR_GENERATOR

    return bag, gens, TERMINATOR_MODULATOR


def build(path, samples, instruments, presets,
          extra_info=b'', extra_pdta=b'', comments=b'Malformed fixture'):
    """Writes one fixture.

    `instruments` is a list of (name, zone_lists) and `presets` a list of
    (name, bank, patch, zone_lists); an instrument whose zone list is empty
    gets a bag span of zero, which is the defect Timbres of Heaven carries.
    """
    shdr = b''.join(shdr_record(*sample) for sample in samples)
    shdr += shdr_record('EOS', 0, 0, 0, 0, 0)

    instrument_zones = [zones for _, zones in instruments]
    ibag, igen, imod = bag_and_records(instrument_zones)

    inst = b''
    zone_index = 0
    for name, zones in instruments:
        inst += name20(name) + struct.pack('<H', zone_index)
        zone_index += len(zones)
    inst += name20('EOI') + struct.pack('<H', zone_index)

    preset_zones = [zones for _, _, _, zones in presets]
    pbag, pgen, pmod = bag_and_records(preset_zones)

    phdr = b''
    zone_index = 0
    for name, bank, patch, zones in presets:
        phdr += (name20(name) + struct.pack('<HHH', patch, bank, zone_index)
                 + struct.pack('<III', 0, 0, 0))
        zone_index += len(zones)
    phdr += (name20('EOP') + struct.pack('<HHH', 255, 255, zone_index)
             + struct.pack('<III', 0, 0, 0))

    info = b'INFO'
    info += chunk(b'ifil', struct.pack('<HH', 2, 1))
    info += chunk(b'isng', b'EMU8000' + NUL)
    info += chunk(b'INAM', b'Malformed fixture' + NUL)
    info += chunk(b'ICMT', comments)
    info += extra_info

    sdta = b'sdta' + chunk(b'smpl', WAVE_DATA)

    pdta = b'pdta'
    pdta += chunk(b'phdr', phdr)
    pdta += chunk(b'pbag', pbag)
    pdta += chunk(b'pmod', pmod)
    pdta += chunk(b'pgen', pgen)
    pdta += chunk(b'inst', inst)
    pdta += chunk(b'ibag', ibag)
    pdta += chunk(b'imod', imod)
    pdta += chunk(b'igen', igen)
    pdta += chunk(b'shdr', shdr)
    pdta += extra_pdta

    body = b'sfbk' + chunk(b'LIST', info) + chunk(b'LIST', sdta) + chunk(b'LIST', pdta)
    sf2 = b'RIFF' + struct.pack('<I', len(body)) + body

    with io.open(path, 'wb') as f:
        f.write(sf2)

    print('wrote %s, %d bytes' % (path, len(sf2)))


GOOD_SAMPLE = ('Good', 0, 100, 8, 92, 60)
# The loop end addresses past the 256 samples the `smpl` chunk holds. This is
# the single defect that used to cost Crisis General Midi its entire file.
BAD_SAMPLE = ('Bad', 0, 100, 8, 300, 62)
SECOND_SAMPLE = ('Second', 128, 228, 136, 220, 64)

ZONE_ON_SAMPLE_0 = [(INITIAL_ATTENUATION, 100), (SAMPLE_ID, 0)]
ZONE_ON_SAMPLE_1 = [(INITIAL_ATTENUATION, 100), (SAMPLE_ID, 1)]
# A zone with no sampleID at all, which used to bind silently to sample 0.
ZONE_WITHOUT_SAMPLE = [(INITIAL_ATTENUATION, 200)]


def preset_on(instrument_id):
    return [(INSTRUMENT, instrument_id)]


# One region plays, the other names a sample whose loop runs off the end.
build('samples/test_bad_region.sf2',
      [GOOD_SAMPLE, BAD_SAMPLE],
      [('TwoRegions', [ZONE_ON_SAMPLE_0, ZONE_ON_SAMPLE_1])],
      [('Preset', 0, 0, [preset_on(0)])])

# The middle instrument has an empty bag span. The third has to keep both its
# regions and its position, since preset regions address instruments by index.
build('samples/test_empty_instrument.sf2',
      [GOOD_SAMPLE, SECOND_SAMPLE],
      [('First', [ZONE_ON_SAMPLE_0]), ('Empty', []), ('Third', [ZONE_ON_SAMPLE_1])],
      [('Preset', 0, 0, [preset_on(0), preset_on(2)])])

# A global zone, one real zone, and one that names no sample.
build('samples/test_zone_without_sample.sf2',
      [GOOD_SAMPLE],
      [('Instrument', [ZONE_WITHOUT_SAMPLE, ZONE_ON_SAMPLE_0, ZONE_WITHOUT_SAMPLE])],
      [('Preset', 0, 0, [preset_on(0)])])

# Nothing playable survives, which is still an error.
build('samples/test_no_usable_region.sf2',
      [BAD_SAMPLE],
      [('Instrument', [ZONE_ON_SAMPLE_0])],
      [('Preset', 0, 0, [preset_on(0)])])

# A vendor chunk in INFO and another in pdta, both of which used to refuse the
# whole bank.
build('samples/test_unknown_chunk.sf2',
      [GOOD_SAMPLE],
      [('Instrument', [ZONE_ON_SAMPLE_0])],
      [('Preset', 0, 0, [preset_on(0)])],
      extra_info=chunk(b'IDBG', b'vendor tag' + NUL),
      extra_pdta=chunk(b'zzzz', b'vendor data'))

# An odd-length ICMT, followed by the pad byte RIFF requires and nothing used
# to consume.
build('samples/test_odd_chunk.sf2',
      [GOOD_SAMPLE],
      [('Instrument', [ZONE_ON_SAMPLE_0])],
      [('Preset', 0, 0, [preset_on(0)])],
      comments=b'odd' + NUL)
