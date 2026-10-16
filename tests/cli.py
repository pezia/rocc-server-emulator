"""Native CLI input protection and real terminal restoration checks."""
import fcntl,hashlib,json,os,pty,resource,select,signal,struct,subprocess,tempfile,termios,time,unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
BINARY=Path(os.environ.get('ROCC_TEST_BINARY',ROOT/'target/release/rocc-emulator')).resolve()

def run(*args,**kwargs):
    kwargs.setdefault('timeout',15)
    return subprocess.run(*args,**kwargs)

def writing_image():
    """Boot, replace RAM word 0 with HALT, then write the boot block to disk."""
    raw=bytearray(6144)
    words={127:0,100:0x1000,101:0,102:0,103:0x663f}
    words.update(enumerate([0x2067,0x4000,0x2064,0x621b,0x2065,0x641b,0x2066,0x665b,0x663f]))
    for address,word in words.items():raw[address*2:address*2+2]=word.to_bytes(2,'big')
    return bytes(raw)

def retaining_image(loop=False):
    raw=bytearray(6144)
    words={127:0,0:0x2464,1:0x8300,2:0x4464,3:3 if loop else 0x663f,100:10941}
    for address,word in words.items():raw[2*address:2*address+2]=word.to_bytes(2,'big')
    return bytes(raw)

class MemoryCliTests(unittest.TestCase):
    def boot(self,image,*args,**kwargs):
        return run([BINARY,'boot',image,'--steps','20',*args],capture_output=True,text=True,**kwargs)
    def test_default_memory_changes_next_boot(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=retaining_image();image.write_bytes(raw)
            for count in (1,2,3):
                report=Path(tmp)/f'{count}.json';p=self.boot(image,'--report',report)
                self.assertEqual(p.returncode,0,p.stderr);r=json.loads(report.read_text())
                self.assertEqual(r['state']['ac'][0],count)
                self.assertEqual(r['memory_persistence']['restored'],count>1)
                self.assertTrue(r['memory_persistence']['committed'])
            self.assertEqual(image.read_bytes(),raw)
    def test_custom_overlay_pairs_memory(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image());overlay=Path(tmp)/'custom'
            self.assertEqual(self.boot(image,'--overlay',overlay).returncode,0)
            self.assertTrue(Path(str(overlay)+'.ram').exists())
            self.assertFalse(Path(str(image)+'.overlay.ram').exists())
    def test_no_memory_skips_load_and_save(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image());ram=Path(str(image)+'.overlay.ram')
            p=self.boot(image,'--no-memory');self.assertEqual(p.returncode,0,p.stderr);self.assertFalse(ram.exists())
            self.assertEqual(self.boot(image).returncode,0);saved=ram.read_bytes();report=Path(tmp)/'disabled.json'
            p=self.boot(image,'--no-memory','--report',report);self.assertEqual(p.returncode,0,p.stderr)
            r=json.loads(report.read_text());self.assertEqual(r['state']['ac'][0],1);self.assertFalse(r['memory_persistence']['enabled']);self.assertEqual(ram.read_bytes(),saved)
    def test_stateless_disk_write_invalidates_saved_ram(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(6144)
            words={127:0,100:1,101:128,102:0,103:0x1001}
            words.update(enumerate([0x2064,0x621b,0x2065,0x641b,0x3866,0x7e5b,0x2080,0x8300,0x4080,0x2067,0x621b,0x7e5b,0x663f]))
            for address,word in words.items():raw[2*address:2*address+2]=word.to_bytes(2,'big')
            image.write_bytes(raw);self.assertEqual(self.boot(image).returncode,0)
            ram=Path(str(image)+'.overlay.ram');saved=ram.read_bytes()
            p=self.boot(image,'--no-memory');self.assertEqual(p.returncode,0,p.stderr)
            p=self.boot(image);self.assertNotEqual(p.returncode,0);self.assertIn('disk state mismatch',p.stderr);self.assertEqual(ram.read_bytes(),saved)
    def test_cold_boot_replaces_recognized_bad_ram(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image());self.assertEqual(self.boot(image,'--profile','extended').returncode,0)
            ram=Path(str(image)+'.overlay.ram');bad=bytearray(ram.read_bytes());bad[160]^=1;ram.write_bytes(bad)
            p=self.boot(image,'--profile','extended');self.assertNotEqual(p.returncode,0);self.assertIn('checksum mismatch',p.stderr)
            report=Path(tmp)/'cold.json';p=self.boot(image,'--profile','extended','--cold-boot','--report',report)
            self.assertEqual(p.returncode,0,p.stderr);r=json.loads(report.read_text());self.assertFalse(r['memory_persistence']['restored']);self.assertTrue(r['memory_persistence']['bypassed_for_cold_boot']);self.assertTrue(r['memory_persistence']['committed'])
    def test_report_alias_is_rejected_before_memory_begin(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image());ram=Path(str(image)+'.overlay.ram')
            p=self.boot(image,'--report',ram);self.assertNotEqual(p.returncode,0);self.assertIn('separate paths',p.stderr);self.assertFalse(ram.exists())
    def test_sidecar_permissions_under_permissive_umask(self):
        for mask in (0,0o022):
            with self.subTest(mask=mask),tempfile.TemporaryDirectory() as tmp:
                image=Path(tmp)/'disk';image.write_bytes(retaining_image())
                for _ in range(2):
                    p=self.boot(image,umask=mask);self.assertEqual(p.returncode,0,p.stderr)
                    self.assertEqual(Path(str(image)+'.overlay.ram').stat().st_mode&0o777,0o600)
    def test_profile_mismatch_preserves_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image());self.assertEqual(self.boot(image).returncode,0)
            ram=Path(str(image)+'.overlay.ram');saved=ram.read_bytes()
            p=self.boot(image,'--profile','extended');self.assertNotEqual(p.returncode,0)
            self.assertIn('geometry',p.stderr);self.assertEqual(ram.read_bytes(),saved)
    def test_save_failure_is_not_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image())
            def limit():
                resource.setrlimit(resource.RLIMIT_FSIZE,(200,200));signal.signal(signal.SIGXFSZ,signal.SIG_IGN)
            p=self.boot(image,preexec_fn=limit);self.assertNotEqual(p.returncode,0);self.assertIn('memory',p.stderr)
            self.assertEqual(Path(str(image)+'.overlay.ram').read_bytes()[10],1)
    def wait_active(self,ram,process):
        deadline=time.monotonic()+5
        while time.monotonic()<deadline:
            if ram.exists() and len(raw:=ram.read_bytes())>=160 and raw[10]==1:return
            if process.poll() is not None:self.fail(repr(process.communicate(timeout=5)))
            time.sleep(.005)
        self.fail('RAM active marker not observed')
    def test_killed_session_without_disk_writes_requires_recovery(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image(loop=True));self.assertEqual(self.boot(image,'--profile','extended').returncode,0)
            ram=Path(str(image)+'.overlay.ram');overlay=Path(str(image)+'.overlay');saved_disk=overlay.read_bytes()
            p=subprocess.Popen([BINARY,image,'--profile','extended','--steps','100000000'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:self.wait_active(ram,p)
            finally:
                if p.poll() is None:p.kill()
                p.communicate(timeout=5)
            self.assertEqual(overlay.read_bytes(),saved_disk)
            saved=ram.read_bytes();p=self.boot(image,'--profile','extended');self.assertNotEqual(p.returncode,0);self.assertIn('interrupted',p.stderr);self.assertEqual(ram.read_bytes(),saved)
            p=self.boot(image,'--profile','extended','--cold-boot');self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(ram.read_bytes()[10],0)
    def test_report_failure_does_not_lose_ram_save(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(retaining_image(loop=True));report=Path(tmp)/'report';ram=Path(str(image)+'.overlay.ram')
            p=subprocess.Popen([BINARY,image,'--steps','10000000','--report',report],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:
                self.wait_active(ram,p);report.write_text('keep this file');out,err=p.communicate(timeout=15)
                self.assertNotEqual(p.returncode,0,repr(out));self.assertIn(b'destination already exists',err)
                self.assertEqual(report.read_text(),'keep this file');self.assertEqual(ram.read_bytes()[10],0)
            finally:
                if p.poll() is None:p.kill();p.communicate(timeout=5)

class CliTests(unittest.TestCase):
    def boot(self,image,*args):
        return run([BINARY,'boot',image,'--steps','20',*args],capture_output=True,text=True)

    def test_cold_boot_runs_guest_panel_handshake_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw=bytearray(6144)
            # Relocated WMS panel sequence: F0F0, HALT, FFFF, continue.
            words={127:40,10:0xf0f0,56:0x663f}
            words.update(enumerate([
                0x613f,0x29e1,0x8d0c,0x010d,0x663f,
                0x613f,0x8004,0x01fd,0x663f,
            ],40))
            for address,word in words.items():raw[address*2:address*2+2]=word.to_bytes(2,'big')
            image=Path(tmp)/'disk';image.write_bytes(raw)
            for cold in [False,True]:
                report=Path(tmp)/f'{cold}.json'
                args=['--cold-boot'] if cold else []
                p=self.boot(image,'--profile','extended','--report',report,*args)
                self.assertEqual(p.returncode,0,p.stderr)
                r=json.loads(report.read_text())
                self.assertEqual(r['configuration']['cold_boot'],cold)
                self.assertEqual(r['cold_boot_completed'],cold)
                self.assertEqual(r['reason'],'halt')
                self.assertEqual(r['state']['pc'],49 if cold else 57)
                self.assertEqual(r['steps'],8 if cold else 6)
                self.assertEqual(r['disk_writes'],0)
            self.assertEqual(image.read_bytes(),raw)

    def test_cold_boot_preserves_unrelated_halt(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw=bytearray(6144);raw[254:256]=(0x663f).to_bytes(2,'big')
            image=Path(tmp)/'disk';image.write_bytes(raw);report=Path(tmp)/'halt.json'
            p=self.boot(image,'--profile','extended','--cold-boot','--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            r=json.loads(report.read_text())
            self.assertEqual((r['reason'],r['steps'],r['state']['pc']),('halt',1,128))
            self.assertFalse(r['cold_boot_completed'])

    def test_cold_boot_requires_extended_profile_before_opening_overlay(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144))
            p=self.boot(image,'--cold-boot')
            self.assertNotEqual(p.returncode,0)
            self.assertIn('cold boot requires extended profile',p.stderr)
            self.assertFalse(Path(str(image)+'.overlay').exists())

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
            p=run([BINARY,'boot',image,'--steps','20','--no-memory'],capture_output=True,text=True,preexec_fn=limit_writes)
            self.assertNotEqual(p.returncode,0)
            self.assertIn('disk_overlay_write',p.stderr)
            self.assertIn('0 writes',p.stdout)
            self.assertEqual(image.read_bytes(),original)
            # Retry recovers the incomplete failed append and completes the write.
            report=Path(tmp)/'retry.json'
            p=self.boot(image,'--no-memory','--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(report.read_text())['disk_writes'],1)

    def test_completed_write_survives_process_kill(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(writing_image())
            raw[16:18]=(8).to_bytes(2,'big') # Loop after the write instead of exiting.
            image.write_bytes(raw);overlay=Path(str(image)+'.overlay')
            p=subprocess.Popen([BINARY,'boot',image,'--steps','100000000','--no-memory'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            try:
                deadline=time.monotonic()+5
                while time.monotonic()<deadline:
                    if overlay.exists() and overlay.stat().st_size>48:break
                    if p.poll() is not None:self.fail(repr(p.communicate(timeout=15)))
                    time.sleep(.005)
                else:self.fail('guest did not persist its write')
            finally:
                p.kill();p.communicate(timeout=5)
            report=Path(tmp)/'resumed.json'
            p=self.boot(image,'--no-memory','--report',report)
            self.assertEqual(p.returncode,0,p.stderr)
            self.assertEqual(json.loads(report.read_text())['steps'],2)
            self.assertEqual(image.read_bytes(),raw)

    def test_explicit_image_required(self):
        p=run([BINARY,'boot'],capture_output=True,text=True)
        self.assertNotEqual(p.returncode,0)
        self.assertIn('image path is required',p.stderr)

    def test_keymap_validation_precedes_terminal_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'keys.json';path.write_text('{"Up":"zz"}')
            p=run([BINARY,'--interactive','--keymap',path],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0);self.assertIn('invalid byte',p.stderr)
            path.write_text('{"F1":"95"}')
            p=run([BINARY,'--keymap',path],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0);self.assertIn('requires --interactive',p.stderr)

    def test_rejects_report_symlink_and_bad_checksum(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144))
            report=Path(tmp)/'report';report.symlink_to(Path(tmp)/'absent')
            p=run([BINARY,'boot',image,'--steps','1','--report',report],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertTrue(report.is_symlink())
            report.unlink()
            p=run([BINARY,'boot',image,'--steps','1','--expected-sha256','0'*64,'--report',report],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertFalse(report.exists())
    def test_invalid_image_and_existing_report_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6145))
            p=run([BINARY,'boot',image],capture_output=True)
            self.assertNotEqual(p.returncode,0)
            image.write_bytes(bytes(6144));report=Path(tmp)/'report';report.write_text('keep')
            p=run([BINARY,'boot',image,'--steps','1','--report',report],capture_output=True)
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
                if p is not None and p.poll() is None:p.kill();p.wait(timeout=5)
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
                    self.assertTrue(json.loads(report.read_text())['memory_persistence']['committed'])
                    self.assertEqual(Path(str(image)+'.overlay.ram').read_bytes()[10],0)
                finally:
                    if p is not None and p.poll() is None:p.kill();p.wait(timeout=5)
                    termios.tcsetattr(slave,termios.TCSANOW,original);os.close(master);os.close(slave)

    def test_report_permissions_ignore_permissive_umask(self):
        for mask in (0,0o022):
            with self.subTest(umask=mask),tempfile.TemporaryDirectory() as tmp:
                image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
                p=run([BINARY,image,'--steps','1','--report',report],capture_output=True,umask=mask,timeout=15)
                self.assertEqual(p.returncode,0,p.stderr)
                self.assertEqual(report.stat().st_mode&0o777,0o600)

    def test_compaction_preserves_source_and_boot_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image());p=self.boot(image);self.assertEqual(p.returncode,0,p.stderr)
            source=Path(str(image)+'.overlay');saved=source.read_bytes();source.write_bytes(saved+saved[48:]*3+b'partial')
            before=source.read_bytes();dest=Path(tmp)/'compact'
            p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',dest],capture_output=True,text=True,timeout=15)
            self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(source.read_bytes(),before);self.assertEqual(image.read_bytes(),writing_image())
            self.assertEqual(dest.stat().st_size,48+296);self.assertEqual(dest.stat().st_mode&0o777,0o600)
            report=Path(tmp)/'report';p=self.boot(image,'--overlay',dest,'--report',report);self.assertEqual(p.returncode,0,p.stderr);self.assertEqual(json.loads(report.read_text())['steps'],2)

    def test_compaction_honors_lock_and_rejects_aliases(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image());self.assertEqual(self.boot(image).returncode,0)
            source=Path(str(image)+'.overlay');before=source.read_bytes();dest=Path(tmp)/'new'
            with source.open('rb') as f:
                fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
                p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',dest],capture_output=True,timeout=15)
                self.assertNotEqual(p.returncode,0);self.assertFalse(dest.exists())
            for out in (source,image):
                p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',out],capture_output=True,timeout=15)
                self.assertNotEqual(p.returncode,0)
            self.assertEqual(source.read_bytes(),before);self.assertEqual(image.read_bytes(),writing_image())

    def test_compaction_rejects_corrupt_source_without_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(writing_image());self.assertEqual(self.boot(image).returncode,0)
            source=Path(str(image)+'.overlay');raw=bytearray(source.read_bytes());raw[-1]^=1;source.write_bytes(raw);dest=Path(tmp)/'new'
            p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',dest],capture_output=True,timeout=15)
            self.assertNotEqual(p.returncode,0);self.assertFalse(dest.exists());self.assertEqual(source.read_bytes(),raw)

    def test_scheduled_key_queue_fault_still_publishes_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(6144);raw[254:256]=(127).to_bytes(2,'big');image.write_bytes(raw);report=Path(tmp)/'report'
            p=run([BINARY,image,'--profile','extended','--terminal','experimental','--steps','3','--connect-after','1','--event-limit','1','--key-at','1:41','--report',report],capture_output=True,text=True)
            self.assertNotEqual(p.returncode,0);self.assertTrue(report.exists(),p.stderr)
            data=json.loads(report.read_text());self.assertEqual(data['reason'],'terminal_event_limit');self.assertEqual(data['steps'],1)

    def test_signal_with_full_output_queue_exits_without_summary_block(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
            master,slave=pty.openpty();p=None;original=termios.tcgetattr(slave)
            try:
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',25,65535,0,0))
                p=subprocess.Popen([BINARY,image,'--interactive','--report',report],stdin=slave,stdout=slave,stderr=slave)
                deadline=time.monotonic()+5
                while termios.tcgetattr(slave)==original and p.poll() is None and time.monotonic()<deadline:time.sleep(.01)
                self.assertNotEqual(termios.tcgetattr(slave),original)
                time.sleep(.1);p.send_signal(signal.SIGTERM)
                self.assertEqual(p.wait(timeout=3),143)
                self.assertEqual(termios.tcgetattr(slave),original);self.assertTrue(report.exists())
            finally:
                if p is not None and p.poll() is None:p.kill();p.wait(timeout=5)
                termios.tcsetattr(slave,termios.TCSANOW,original);os.close(master);os.close(slave)

    def test_interactive_key_queue_fault_retains_outcome_and_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';raw=bytearray(6144)
            for address,word in {127:0,0:0x2064,1:0x6228,2:0x6428,3:0x2065,4:0x6628,5:5,100:0,101:65}.items():raw[2*address:2*address+2]=word.to_bytes(2,'big')
            image.write_bytes(raw);report=Path(tmp)/'report';master,slave=pty.openpty();p=None;original=termios.tcgetattr(slave)
            try:
                fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
                p=subprocess.Popen([BINARY,image,'--interactive','--connect-after','1','--event-limit','2','--report',report],stdin=slave,stdout=slave,stderr=slave)
                out=bytearray();deadline=time.monotonic()+5
                while b'\x1b[1;1HA' not in out and p.poll() is None and time.monotonic()<deadline:
                    if select.select([master],[],[],.05)[0]:out.extend(os.read(master,65536))
                self.assertIn(b'\x1b[1;1HA',out);os.write(master,b'B')
                while p.poll() is None and time.monotonic()<deadline:
                    if select.select([master],[],[],.05)[0]:out.extend(os.read(master,65536))
                self.assertEqual(p.poll(),1,repr(out[-500:]));data=json.loads(report.read_text())
                self.assertEqual(data['reason'],'terminal_event_limit');self.assertEqual(data['interactive']['keys_pending'],1)
                self.assertEqual(data['interactive']['keys_delivered'],0);self.assertEqual(data['interactive']['delivered_keys'],[])
                self.assertEqual(termios.tcgetattr(slave),original)
            finally:
                if p is not None and p.poll() is None:p.kill();p.wait(timeout=5)
                termios.tcsetattr(slave,termios.TCSANOW,original);os.close(master);os.close(slave)

    def test_invalid_limits_leave_runtime_artifacts_absent(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';image.write_bytes(bytes(6144));report=Path(tmp)/'report'
            for option,value in [('--event-limit','0'),('--event-limit','65537'),('--capture-bytes','67108865'),('--packet-limit','65537')]:
                p=run([BINARY,image,option,value,'--report',report],capture_output=True,text=True)
                self.assertNotEqual(p.returncode,0);self.assertIn('invalid resource limits',p.stderr)
                self.assertFalse(report.exists());self.assertFalse(Path(str(image)+'.overlay').exists())

    def test_compaction_wrong_base_and_failed_publication_preserve_sources(self):
        with tempfile.TemporaryDirectory() as tmp:
            image=Path(tmp)/'disk';original=writing_image();image.write_bytes(original);self.assertEqual(self.boot(image).returncode,0)
            source=Path(str(image)+'.overlay');before=source.read_bytes();dest=Path(tmp)/'new'
            changed=bytearray(original);changed[-1]^=1;image.write_bytes(changed)
            p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',dest],capture_output=True)
            self.assertNotEqual(p.returncode,0);self.assertFalse(dest.exists());self.assertEqual(source.read_bytes(),before);self.assertEqual(image.read_bytes(),changed)
            image.write_bytes(original)
            def limit_writes():
                resource.setrlimit(resource.RLIMIT_FSIZE,(100,100));signal.signal(signal.SIGXFSZ,signal.SIG_IGN)
            p=run([BINARY,'compact-overlay',image,'--overlay',source,'--output',dest],capture_output=True,preexec_fn=limit_writes)
            self.assertNotEqual(p.returncode,0);self.assertFalse(dest.exists());self.assertEqual(source.read_bytes(),before);self.assertEqual(image.read_bytes(),original)
            self.assertFalse(list(Path(tmp).glob('.rocc-artifact-*')))

if __name__=='__main__':unittest.main()
