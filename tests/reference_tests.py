"""Hand-calculated checks of the independent synthetic reference model."""
import unittest
from reference_model import evaluate

def case(word,ac,carry=0,bits=16,memory=None,steps=1):
    return dict(extended=False,address_bits=bits,pc=100,carry=carry,ac=ac,memory=[[100,word],*(memory or [])],steps=steps)
class ModelTests(unittest.TestCase):
    def test_add_wrap_carry(self):
        out=evaluate(case(0x8600,[65535,0,0,0]))
        self.assertEqual(out['state'],dict(pc=101,ac=[65534,0,0,0],carry=1))
    def test_rotate_through_carry(self):
        self.assertEqual(evaluate(case(0x8280,[1,0,0,0],1))['state'],dict(pc=101,ac=[32768,0,0,0],carry=1))
    def test_no_load_skip_preserves_registers_and_carry(self):
        self.assertEqual(evaluate(case(0x8209,[42,0,0,0],1))['state'],dict(pc=102,ac=[42,0,0,0],carry=1))
    def test_relative_negative(self):
        self.assertEqual(evaluate(case(0x21ff,[0,0,0,0],memory=[[99,123]]))['state']['ac'][0],123)
    def test_autoindex_original_continuation(self):
        c=case(0x2410,[0,0,0,0],bits=15,memory=[[16,65535],[0,0x2410],[0x2410,0x7777]])
        out=evaluate(c);self.assertEqual(out['state']['ac'][0],0x7777);self.assertIn([16,0],out['memory'])
if __name__=='__main__':unittest.main()
