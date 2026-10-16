#!/usr/bin/env python3
"""Generate the new independent base-ISA corpus; never touch the frozen corpus."""
import argparse,json,random
from pathlib import Path
from reference_model import evaluate,MODEL_VERSION
SEED=1380926275
DEST=Path(__file__).with_name('generated-reference.json')

def generate(seed=SEED):
    rng=random.Random(seed);inputs=[]
    for n in range(512):
        word=rng.randrange(0x8000,0x10000)
        # Exclude all recovered/reserved extension encodings in both profiles.
        while word&15==8 or word in (0xabc9,0xbbc9,0xfbc9):word=rng.randrange(0x8000,0x10000)
        inputs.append(dict(extended=bool(n&1),address_bits=16,pc=100,carry=rng.randrange(2),ac=[rng.randrange(65536) for _ in range(4)],memory=[[100,word]],steps=1))
    for n in range(64):
        words=[]
        for _ in range(16):
            w=rng.randrange(0x8000,0x10000)
            # No skipping: every program reaches its final HALT.
            w=(w&~15)
            words.append(w)
        words.append(0x663f)
        inputs.append(dict(extended=False,address_bits=15 if n&1 else 16,pc=100,carry=n&1,ac=[rng.randrange(65536) for _ in range(4)],memory=[[100+i,w] for i,w in enumerate(words)],steps=20))
    for bits in (15,16):
        for mode in range(4):
            for disp in (0,1,127,128,255):
                address=disp if mode==0 else (100+(disp if disp<128 else disp-256))&((1<<bits)-1)
                if address==100:continue
                inputs.append(dict(extended=False,address_bits=bits,pc=100,carry=1,ac=[0,0,100,100],memory=[[100,0x2000|(mode<<8)|disp],[address,0x1234]],steps=1))
    return dict(model_version=MODEL_VERSION,seed=seed,provenance='Independent Python model of documented base ISA; no hardware/vendor oracle',cases=[dict(input=c,expected=evaluate(c)) for c in inputs])

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args()
    raw=json.dumps(generate(),sort_keys=True,indent=2)+'\n'
    if args.check:
        if not DEST.exists() or DEST.read_text()!=raw:raise SystemExit('generated reference differs; run python3 tests/generate_reference.py')
        print('independent reference is reproducible')
    else:DEST.write_text(raw)
if __name__=='__main__':main()
