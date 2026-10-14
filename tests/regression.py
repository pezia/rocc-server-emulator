#!/usr/bin/env python3
"""Check synthetic CPU cases against frozen reference results.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REFERENCE_SHA256 = 'f0e281511050716f1ea3f8b53f321b3bd896f7a209b264427d5a7df68651ae07'

def check(expected, actual, label):
    if expected != actual:
        raise AssertionError(f'{label}: expected {expected!r}, got {actual!r}')

def cpu(binary):
    raw = gzip.decompress((ROOT/'tests/cpu-reference.json.gz').read_bytes())
    check(REFERENCE_SHA256, hashlib.sha256(raw).hexdigest(), 'fixture SHA256')
    cases = json.loads(raw)['cases']
    result = subprocess.run([str(binary), 'cpu-cases'], input=json.dumps([c['input'] for c in cases]),
                            text=True, capture_output=True, check=True)
    actual = json.loads(result.stdout)
    check(len(cases), len(actual), 'case count')
    for n, (case, result) in enumerate(zip(cases, actual)):
        check(case['expected'], result, f'CPU case {n}')
    return {'cpu_cases': len(cases)}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cpu',action='store_true')
    parser.add_argument('--binary',type=Path,default=ROOT/'target/release/rocc-emulator')
    args=parser.parse_args()
    if not args.cpu:
        parser.error('select --cpu')
    result={}
    if args.cpu: result.update(cpu(args.binary.resolve()))
    print(json.dumps(result,sort_keys=True))

if __name__=='__main__':
    main()
