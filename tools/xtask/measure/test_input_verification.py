"""Seed and independent-model input checks on a tiny generated corpus."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_schedule
import gen_inputs
import input_verification


class InputVerificationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'inputs'
        streams = {name: (0, 0) for _, name in counter_schedule.KERNELS}
        cells = [(kernel, stream, 1, 2) for kernel, stream in counter_schedule.KERNELS]
        self.patches = [patch.object(gen_inputs, 'K_MAX', 2), patch.object(gen_inputs, 'STREAMS', streams),
                        patch.object(gen_inputs, 'CELLS', cells)]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)
        gen_inputs.main(str(self.root), 1)

    def test_generated_streams_and_manifest_reproduce(self):
        result = input_verification.verify(self.root)
        self.assertEqual(result['seed'], 1)
        self.assertEqual(result['streams'], 4)
        (self.root / 'loop_main.inputs.txt').write_text('1\n0\n')
        with self.assertRaisesRegex(ValueError, 'differs from seed'):
            input_verification.verify(self.root)

    def test_forged_model_checksum_is_rejected(self):
        path = self.root / 'MANIFEST.json'
        manifest = json.loads(path.read_text())
        manifest['cells']['fib/fib_main']['expected_checksum']['1'] = 1
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'differs from seed and model'):
            input_verification.verify(self.root)


if __name__ == '__main__':
    unittest.main()
