"""Analysis manifests bind synthetic current-run files and analyzer code."""
import json
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_package
import test_callgrind_packet


class AnalysisPackageTests(unittest.TestCase):
    def setUp(self):
        fixture = test_callgrind_packet.CallgrindPacketTests('test_complete_phases_in_isolated_package')
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root, self.spec = fixture.root, fixture.spec
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))

    def test_manifest_rechecks_before_analysis(self):
        manifest = analysis_package.inspect(self.root, 'callgrind-cgu1-v1')
        result = analysis_package.analyze(self.root, manifest)
        self.assertEqual(result['off']['cells']['while']['rule'], 'none')
        altered = deepcopy(manifest)
        altered['orchestrator']['files'][0]['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'differ from package manifest'):
            analysis_package.analyze(self.root, altered)
        altered = deepcopy(manifest)
        altered['runtime']['stdlib']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'differ from package manifest'):
            analysis_package.analyze(self.root, altered)
        (self.root / self.spec['phases']['off']['cells']['while']['tip']['profile']).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'differ from package manifest'):
            analysis_package.analyze(self.root, manifest)

    def test_declared_optional_file_is_required(self):
        self.spec['symbol_map'] = 'SYMBOL-NAME-MAP.json'
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        with self.assertRaises(ValueError):
            analysis_package.inspect(self.root, 'callgrind-cgu1-v1')
        with self.assertRaisesRegex(ValueError, 'unknown analysis protocol'):
            analysis_package.inspect(self.root, 'unknown')


if __name__ == '__main__':
    unittest.main()
