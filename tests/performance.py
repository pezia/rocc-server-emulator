#!/usr/bin/env python3
"""Synthetic measurements; no timing thresholds or private images required."""
import argparse,hashlib,json,platform,shutil,subprocess,tempfile,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def timed(command):
    with tempfile.TemporaryDirectory() as tmp:
        metrics=Path(tmp)/'metrics';timer=shutil.which('/usr/bin/time')
        cmd=[timer,'-f','%M','-o',str(metrics),*map(str,command)] if timer else list(map(str,command))
        start=time.perf_counter();p=subprocess.run(cmd,text=True,capture_output=True,check=True,timeout=120)
        return dict(wall_seconds=time.perf_counter()-start,peak_kib=int(metrics.read_text()) if timer else None,stdout=p.stdout.strip())
def replay(binary):
    with tempfile.TemporaryDirectory() as tmp:
        d=Path(tmp);image=d/'disk';raw=bytes(6144);image.write_bytes(raw);source=d/'source';output=d/'compact'
        header=b'ROCCOVL1'+len(raw).to_bytes(8,'big')+hashlib.sha256(raw).digest()
        records=[]
        for n in range(20000):
            r=((n%8+1)*256).to_bytes(8,'big')+bytes(256);records.append(r+hashlib.sha256(r).digest())
        source.write_bytes(header+b''.join(records))
        before=timed([binary,'boot',image,'--overlay',source,'--steps','1'])
        compact=timed([binary,'compact-overlay',image,'--overlay',source,'--output',output])
        after=timed([binary,'boot',image,'--overlay',output,'--steps','1'])
        return dict(before=before,compaction=compact,after=after,source_bytes=source.stat().st_size,compact_bytes=output.stat().st_size)
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--probe',type=Path,default=ROOT/'target/release/examples/performance');parser.add_argument('--binary',type=Path,default=ROOT/'target/release/rocc-emulator');parser.add_argument('--skip-replay',action='store_true');args=parser.parse_args()
    result={'environment':platform.platform(),'probes':{}}
    for case,n in [('instructions',10000000),('terminal-tail',10000),('report',10000)]:
        row=timed([args.probe,case,n]);row['measurement']=json.loads(row.pop('stdout'));result['probes'][case]=row
    if not args.skip_replay:result['replay']=replay(args.binary)
    print(json.dumps(result,sort_keys=True,indent=2))
if __name__=='__main__':main()
