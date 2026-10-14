"""Native CLI input protection and real terminal restoration checks."""
import fcntl,json,os,pty,select,struct,subprocess,tempfile,termios,time,unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
BINARY=ROOT/'target/release/rocc-emulator'

class CliTests(unittest.TestCase):
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

if __name__=='__main__':unittest.main()
