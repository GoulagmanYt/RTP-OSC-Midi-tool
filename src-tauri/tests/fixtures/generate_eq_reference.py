"""Independent reference: analog W3C prototypes evaluated with BLT prewarping.

No production helpers or digital biquad coefficients are used.
https://www.w3.org/TR/audio-eq-cookbook/
"""
import json
import math
from pathlib import Path

def settings(low=(120, 0), mid=(1000, 0, 1), high=(8000, 0)):
    return dict(enabled=True, lowShelf=dict(frequencyHz=low[0], gainDb=low[1]),
                midPeak=dict(frequencyHz=mid[0], gainDb=mid[1], q=mid[2]),
                highShelf=dict(frequencyHz=high[0], gainDb=high[1]))


def response(cfg, rate, frequency):
    h = 1
    for kind, lower, upper in [('lowShelf', 40, 500), ('midPeak', 100, 10000), ('highShelf', 2000, 16000)]:
        band = cfg[kind]
        corner = min(max(band['frequencyHz'], lower), upper, rate * .45)
        gain = min(max(band['gainDb'], -18), 18)
        a = 10 ** (gain / 40)
        s = 1j * math.tan(math.pi * frequency / rate) / math.tan(math.pi * corner / rate)
        if kind == 'midPeak':
            q = min(max(band['q'], .2), 10)
            h *= (s*s + (a/q)*s + 1) / (s*s + s/(a*q) + 1)
        elif kind == 'lowShelf':
            h *= a * (s*s + math.sqrt(2*a)*s + a) / (a*s*s + math.sqrt(2*a)*s + 1)
        else:
            h *= a * (a*s*s + math.sqrt(2*a)*s + 1) / (s*s + math.sqrt(2*a)*s + a)
    return 20 * math.log10(abs(h))


cases = [
    ('low corner at 96k', 96000, settings(low=(40, 18)), [20, 40, 100, 1000, 20000]),
    ('narrow peak at 96k', 96000, settings(mid=(100, 18, 10)), [20, 100, 120, 1000]),
    ('high band clamped at 32k', 32000, settings(high=(16000, 12)), [1000, 8000, 14400, 15500]),
    ('high band clamped at 8k', 8000, settings(high=(8000, 12)), [100, 2000, 3500, 3600, 3900]),
    ('combined at 44k1', 44100, settings(low=(120, -6), mid=(1000, 9, .7), high=(8000, -12)), [20, 120, 1000, 8000, 19000, 20000]),
    ('limits at 48k', 48000, settings(low=(40, -18), mid=(100, -18, .2), high=(16000, 18)), [20, 40, 100, 1000, 16000, 20000]),
]
data = [dict(name=name, sampleRate=rate, settings=cfg,
             response=[dict(frequencyHz=f, db=response(cfg, rate, f)) for f in frequencies])
        for name, rate, cfg, frequencies in cases]
dest = Path(__file__).with_name('eq-response.json')
dest.write_text(json.dumps(data, indent=2) + '\n')
print(f'{len(data)} cases written to {dest.name}')
