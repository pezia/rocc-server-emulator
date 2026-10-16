"""Native CLI input protection and real terminal restoration checks."""
import fcntl,hashlib,json,os,pty,resource,select,signal,struct,subprocess,tempfile,termios,time,unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
BINARY=ROOT/'target/release/rocc-emulator'

def writing_image():
    """Boot, replace RAM word 0 with HALT, then write the boot block to disk."""
    raw=bytearray(6144)
    words={127:0,100:0x1000,101:0,102:0,103:0x663f}
    words.update(enumerate([0x2067,0x4000,0x2064,0x621b,0x2065,0x641b,0x2066,0x665b,0x663f]))
    for address,word in words.items():raw[address*2:address*2+2]=word.to_bytes(2,'big')
    return bytes(raw)

class CliTests(unittest.TestCase):
    def boot(self,image,*args):
        return subprocess.run([BINARY,'boot',image,'--steps','20',*args],capture_output=True,text=True)

    def test_default_overlay_persists_and_boots_saved_sector(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk.img';original=writing_image();image.write_bytes(original)
            first=Path(tmp)/'first.json';second=Path(tmp)/'second.json'
            p=self.boot(image,'--report',first)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(first.read_text())['disk_writes'],1)
            overlay=Path(str(image)+'.overlay')
            self.assertTrue(overlay.is_file())
            saved=overlay.read_bytes()
            p=self.boot(image,'--report',second)
            self.assertEqual(p.returncode,0,p.stderr)
            report=json.loads(second.read_text())
            self.assertEqual((report['reason'],report['steps'],report['disk_writes']),('halt',2,0))
            self.assertEqual(overlay.read_bytes(),saved)
            self.assertEqual(image.read_bytes(),original)

    def test_overlay_override_resumes_without_default_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image());overlay=Path(tmp)/'saved'
            for n in range(2):
                report=Path(tmp)/f'{n}.json'
                p=self.boot(image,'--overlay',overlay,'--report',report)
                self.assertEqual(p.returncode,0,p.stderr)
                self.assertEqual(json.loads(report.read_text())['disk_writes'],1 if n==0 else 0)
            self.assertTrue(overlay.is_file())
            self.assertFalse(Path(str(image)+'.overlay').exists())

    def test_repeated_writes_replay_latest_block(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(writing_image())
            # After the first write, change another word and write the block again.
            for address,word in {8:0x2068,9:0x406e,10:0x2066,11:0x665b,12:0x663f,104:0x1234}.items():
                raw[2*address:2*address+2]=word.to_bytes(2,'big')
            image.write_bytes(raw);first=Path(tmp)/'first.json';second=Path(tmp)/'second.json'
            p=self.boot(image,'--report',first);self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(first.read_text())['disk_writes'],2)
            p=self.boot(image,'--report',second);self.assertEqual(p.returncode,0,p.stderr)
            report=json.loads(second.read_text())
            self.assertEqual(report['steps'],2)
            block=bytes.fromhex(report['overlay']['blocks'][0]['hex'])
            self.assertEqual(block[220:222],b'\x12\x34')
            self.assertEqual(image.read_bytes(),raw)

    def test_persisted_data_sector_is_read_by_disk_dma(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw=bytearray(6144)
            words={127:0,100:1,101:128,102:0}
            words.update(enumerate([0x2064,0x621b,0x2065,0x641b,0x2066,0x665b,0x2080,0x663f]))
            for address,word in words.items():raw[2*address:2*address+2]=word.to_bytes(2,'big')
            image=Path(tmp)/'disk';image.write_bytes(raw)
            header=b'ROCCOVL1'+len(raw).to_bytes(8,'big')+hashlib.sha256(raw).digest()
            record=(256).to_bytes(8,'big')+b'\x12\x34'+bytes(254)
            Path(str(image)+'.overlay').write_bytes(header+record+hashlib.sha256(record).digest())
            report=Path(tmp)/'read.json'
            p=self.boot(image,'--report',report);self.assertEqual(p.returncode,0,p.stderr)
            data=json.loads(report.read_text())
            self.assertEqual((data['disk_reads'],data['disk_writes'],data['state']['ac'][0]),(1,0,0x1234))
            self.assertEqual(image.read_bytes(),raw)

    def test_checksummed_invalid_offsets_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=writing_image();image.write_bytes(raw)
            overlay=Path(str(image)+'.overlay')
            header=b'ROCCOVL1'+len(raw).to_bytes(8,'big')+hashlib.sha256(raw).digest()
            for offset in [1,len(raw),2**64-1]:
                with self.subTest(offset=offset):
                    record=offset.to_bytes(8,'big')+bytes(256)
                    saved=header+record+hashlib.sha256(record).digest();overlay.write_bytes(saved)
                    p=self.boot(image)
                    self.assertNotEqual(p.returncode,0);self.assertIn('overlay',p.stderr)
                    self.assertEqual(overlay.read_bytes(),saved)
                    self.assertEqual(image.read_bytes(),raw)

    def test_overlay_cannot_alias_image_or_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';original=writing_image();image.write_bytes(original)
            hard=Path(tmp)/'hard';os.link(image,hard)
            soft=Path(tmp)/'soft';soft.symlink_to(image)
            for overlay in [image,hard,soft]:
                with self.subTest(overlay=overlay):
                    p=self.boot(image,'--overlay',overlay)
                    self.assertNotEqual(p.returncode,0)
                    self.assertIn('overlay',p.stderr)
                    self.assertEqual(image.read_bytes(),original)

    def test_overlay_rejects_wrong_base_and_corruption_without_rewriting(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';original=writing_image();image.write_bytes(original)
            p=self.boot(image);self.assertEqual(p.returncode,0,p.stderr)
            overlay=Path(str(image)+'.overlay');saved=overlay.read_bytes()
            changed=bytearray(original);changed[-1]=1;image.write_bytes(changed)
            p=self.boot(image)
            self.assertNotEqual(p.returncode,0);self.assertIn('overlay',p.stderr)
            self.assertEqual(overlay.read_bytes(),saved)
            image.write_bytes(original)
            corrupt=bytearray(saved);corrupt[-1]^=1;overlay.write_bytes(corrupt)
            p=self.boot(image)
            self.assertNotEqual(p.returncode,0);self.assertIn('overlay',p.stderr)
            self.assertEqual(overlay.read_bytes(),corrupt)
            self.assertEqual(image.read_bytes(),original)

    def test_incomplete_final_write_preserves_previous_committed_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image())
            p=self.boot(image);self.assertEqual(p.returncode,0,p.stderr)
            overlay=Path(str(image)+'.overlay');saved=overlay.read_bytes()
            with overlay.open('ab') as file:file.write(b'partial record')
            report=Path(tmp)/'resumed.json'
            p=self.boot(image,'--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(report.read_text())['steps'],2)
            self.assertEqual(overlay.read_bytes(),saved)

    def test_overlay_lock_prevents_concurrent_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image())
            p=self.boot(image);self.assertEqual(p.returncode,0,p.stderr)
            overlay=Path(str(image)+'.overlay');saved=overlay.read_bytes()
            with overlay.open('rb') as file:
                fcntl.flock(file,fcntl.LOCK_EX|fcntl.LOCK_NB)
                p=self.boot(image)
                self.assertNotEqual(p.returncode,0);self.assertIn('overlay',p.stderr)
            self.assertEqual(overlay.read_bytes(),saved)

    def test_invalid_overlay_file_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image())
            overlay=Path(str(image)+'.overlay');overlay.write_bytes(b'keep this file')
            p=self.boot(image)
            self.assertNotEqual(p.returncode,0);self.assertIn('overlay',p.stderr)
            self.assertEqual(overlay.read_bytes(),b'keep this file')

    def test_disk_write_failure_is_not_acknowledged(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';original=writing_image();image.write_bytes(original)
            def limit_writes():
                resource.setrlimit(resource.RLIMIT_FSIZE,(100,100))
                signal.signal(signal.SIGXFSZ,signal.SIG_IGN)
            p=subprocess.run([BINARY,'boot',image,'--steps','20'],capture_output=True,text=True,preexec_fn=limit_writes)
            self.assertNotEqual(p.returncode,0)
            self.assertIn('disk_overlay_write',p.stderr)
            self.assertIn('0 writes',p.stdout)
            self.assertEqual(image.read_bytes(),original)
            # Retry recovers the incomplete failed append and completes the write.
            report=Path(tmp)/'retry.json'
            p=self.boot(image,'--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(report.read_text())['disk_writes'],1)

    def test_completed_write_survives_process_kill(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(writing_image())
            raw[16:18]=(8).to_bytes(2,'big') # Loop after the write instead of exiting.
            image.write_bytes(raw);overlay=Path(str(image)+'.overlay')
            p=subprocess.Popen([BINARY,'boot',image,'--steps','100000000'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:
                deadline=time.monotonic()+5
                while time.monotonic()<deadline:
                    if overlay.exists() and overlay.stat().st_size>48:break
                    if p.poll() is not None:self.fail(repr(p.communicate()))
                    time.sleep(.005)
                else:self.fail('guest did not persist its write')
            finally:
                p.kill();p.communicate(timeout=5)
            report=Path(tmp)/'resumed.json'
            p=self.boot(image,'--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(report.read_text())['steps'],2)
            self.assertEqual(image.read_bytes(),raw)

    def test_explicit_image_required(self):
        p=subprocess.run([BINARY,'boot'],capture_output=True,text=True)
        self.assertNotEqual(p.returncode,0)
        self.assertIn('image path is required',p.stderr)

    def test_keymap_validation_precedes_terminal_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'keys.json';path.write_text('{"Up":"zz"}')
            p=subprocess.run([BINARY,'--interactive','--keymap',path],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0);self.assertIn('invalid byte',p.stderr)
            path.write_text('{"F1":"95"}')
            p=subprocess.run([BINARY,'--keymap',path],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0);self.assertIn('requires --interactive',p.stderr)

    def test_rejects_report_symlink_and_bad_checksum(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144))
            report=Path(tmp)/'report';report.symlink_to(Path(tmp)/'absent')
            p=subprocess.run([BINARY,'boot',image,'--steps','1','--report',report],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertTrue(report.is_symlink())
            report.unlink()
            p=subprocess.run([BINARY,'boot',image,'--steps','1','--expected-sha256','0'*64,'--report',report],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertFalse(report.exists())
    def test_invalid_image_and_existing_report_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6145))
            p=subprocess.run([BINARY,'boot',image],capture_output=True)
            self.assertNotEqual(p.returncode,0)
            image.write_bytes(bytes(6144));report=Path(tmp)/'report';report.write_text('keep')
            p=subprocess.run([BINARY,'boot',image,'--steps','1','--report',report],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertEqual(report.read_text(),'keep')
            self.assertEqual(image.read_bytes(),bytes(6144))
    def test_live_terminal_exit_restores_exact_modes(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
            master,slave=pty.openpty();p=None
            try:
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
                original=termios.tcgetattr(slave)
                p=subprocess.Popen([BINARY,image,'--interactive','--report',report],stdin=slave,stdout=slave,stderr=slave)
                out=bytearray();sent=False;deadline=time.monotonic()+10
                while p.poll() is None and time.monotonic()<deadline:
                    if select.select([master],[],[],.05)[0]:out.extend(os.read(master,65536))
                    if not sent and b'Booting guest' in out:os.write(master,b'\x1d');sent=True
                self.assertTrue(sent,repr(out[-500:]));self.assertEqual(p.poll(),0,repr(out[-500:]))
                self.assertEqual(termios.tcgetattr(slave),original)
                self.assertEqual(json.loads(report.read_text())['reason'],'user_exit')
            finally:
                if p is not None and p.poll() is None:p.kill();p.wait()
                os.close(master);os.close(slave)

    def test_guest_fault_restores_terminal(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(6144);raw[254:256]=b'\x00\xc8';image.write_bytes(raw)
            master,slave=pty.openpty()
            try:
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
                original=termios.tcgetattr(slave)
                p=subprocess.Popen([BINARY,image,'--interactive'],stdin=slave,stdout=slave,stderr=slave)
                p.wait(timeout=10)
                self.assertEqual(p.returncode,1);self.assertEqual(termios.tcgetattr(slave),original)
            finally:os.close(master);os.close(slave)

    def test_signals_restore_terminal_and_publish_report(self):
        for sig in (signal.SIGINT,signal.SIGTERM,signal.SIGHUP):
            with self.subTest(signal=sig), tempfile.TemporaryDirectory() as tmp:
                image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
                master,slave=pty.openpty();p=None
                try:
                    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
                    original=termios.tcgetattr(slave)
                    p=subprocess.Popen([BINARY,image,'--interactive','--report',report],stdin=slave,stdout=slave,stderr=slave)
                    out=bytearray();deadline=time.monotonic()+5
                    while b'Booting guest' not in out and time.monotonic()<deadline:
                        if select.select([master],[],[],.05)[0]:out.extend(os.read(master,65536))
                    self.assertIn(b'Booting guest',out)
                    p.send_signal(sig)
                    while p.poll() is None and time.monotonic()<deadline:
                        if select.select([master],[],[],.05)[0]:out.extend(os.read(master,65536))
                    self.assertEqual(p.poll(),128+sig)
                    self.assertEqual(termios.tcgetattr(slave),original)
                    self.assertEqual(json.loads(report.read_text())['interactive']['signal'],sig)
                finally:
                    if p is not None and p.poll() is None:p.kill();p.wait()
                    termios.tcsetattr(slave,termios.TCSANOW,original);os.close(master);os.close(slave)

    def test_report_permissions_ignore_permissive_umask(self):
        for mask in (0,0o022):
            with self.subTest(umask=mask),tempfile.TemporaryDirectory() as tmp:
                image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
                p=subprocess.run([BINARY,image,'--steps','1','--report',report],capture_output=True,umask=mask,timeout=15)
                self.assertEqual(p.returncode,0,p.stderr)
                self.assertEqual(report.stat().st_mode&0o777,0o600)

if __name__=='__main__':unittest.main()
