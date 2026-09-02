"""Builds a minimal SF2 that carries modulators, for use as a committed test fixture.

One preset -> one instrument -> one sample, with modulators in both `pmod` and
`imod`, so the whole load path can be tested with no external font.

Every chunk is built to an even length on purpose, so that this fixture tests
the modulator path and nothing else. Odd chunks and their RIFF pad byte are
`samples/make_test_malformed.py`'s business.
"""
import struct
import io

NUL = bytes([0])


def chunk(fourcc, data):
    assert len(data) % 2 == 0, (fourcc, len(data))
    return fourcc + struct.pack('<I', len(data)) + data


def name20(text):
    return text.encode('ascii')[:20].ljust(20, NUL)


# ---- sdta: a short sample plus the 46 trailing zero samples the spec wants ----
SAMPLE_LEN = 64
sample_data = bytes(2 * (SAMPLE_LEN + 46))


def shdr_record(name, start, end, start_loop, end_loop, rate, pitch, correction, link, kind):
    return (name20(name)
            + struct.pack('<IIIII', start, end, start_loop, end_loop, rate)
            + struct.pack('<BbHH', pitch, correction, link, kind))


shdr = shdr_record('TestSample', 0, SAMPLE_LEN, 8, SAMPLE_LEN - 8, 44100, 60, 0, 0, 1)
shdr += shdr_record('EOS', 0, 0, 0, 0, 0, 0, 0, 0, 0)


def modulator(source, destination, amount, amount_source, transform):
    return struct.pack('<HHhHH', source, destination, amount, amount_source, transform)


TERMINATOR = modulator(0, 0, 0, 0, 0)

# Velocity, concave, unipolar, negative -> initialAttenuation, amount 800.
# An identity match for SF2 default modulator 1, so it has to replace it
# rather than stack with it.
VELOCITY_CONCAVE_NEGATIVE = 0x0502
# CC74, linear, unipolar, positive -> initialFilterFc.
CC74_LINEAR = 0x0080 | 74
# A linked modulator, which has to be dropped at load time.
LINKED_SOURCE = 127

imod = modulator(VELOCITY_CONCAVE_NEGATIVE, 48, 800, 0, 0)
imod += modulator(CC74_LINEAR, 8, 2400, 0, 0)
imod += modulator(LINKED_SOURCE, 48, 100, 0, 0)
imod += TERMINATOR

# A preset modulator, which lands on the preset region only.
CC91_LINEAR = 0x0080 | 91
pmod = modulator(CC91_LINEAR, 16, 350, 0, 0)
pmod += TERMINATOR


def generator(kind, value):
    return struct.pack('<HH', kind, value & 0xFFFF)


SAMPLE_ID = 53
INSTRUMENT = 41
INITIAL_ATTENUATION = 48

igen = generator(INITIAL_ATTENUATION, 100) + generator(SAMPLE_ID, 0) + generator(0, 0)
pgen = generator(INSTRUMENT, 0) + generator(0, 0)

# One real zone, then the terminator that bounds it.
ibag = struct.pack('<HH', 0, 0) + struct.pack('<HH', 2, 3)
pbag = struct.pack('<HH', 0, 0) + struct.pack('<HH', 1, 1)

inst = name20('TestInstrument') + struct.pack('<H', 0)
inst += name20('EOI') + struct.pack('<H', 1)

phdr = name20('TestPreset') + struct.pack('<HHH', 0, 0, 0) + struct.pack('<III', 0, 0, 0)
phdr += name20('EOP') + struct.pack('<HHH', 255, 255, 1) + struct.pack('<III', 0, 0, 0)

info = b'INFO'
info += chunk(b'ifil', struct.pack('<HH', 2, 1))
info += chunk(b'isng', b'EMU8000' + NUL)
info += chunk(b'INAM', b'Modulator fixture' + NUL)

sdta = b'sdta' + chunk(b'smpl', sample_data)

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

body = b'sfbk' + chunk(b'LIST', info) + chunk(b'LIST', sdta) + chunk(b'LIST', pdta)
sf2 = b'RIFF' + struct.pack('<I', len(body)) + body

with io.open('samples/test_modulators.sf2', 'wb') as f:
    f.write(sf2)

print('wrote samples/test_modulators.sf2, %d bytes' % len(sf2))
