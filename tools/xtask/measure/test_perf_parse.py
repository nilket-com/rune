"""Counter parser controls: absent or multiplexed events never become zero."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import perf_parse

EVENTS = {'cycles': ('cycles', ('cycles',)), 'instructions': ('instructions', ('instructions',))}
GROUPS = {'B1': ('instructions', 'cycles')}


class PerfTests(unittest.TestCase):
    def test_available_and_unavailable_counters(self):
        text = '20;;instructions;10;100.00;;\n10;;cycles;10;100.00;;\n'
        result = perf_parse.parse(text, 'B1', EVENTS, GROUPS)
        self.assertTrue(perf_parse.probe_ok(result)[0])
        missing = perf_parse.parse(text.replace('10;;cycles', '<not counted>;;cycles'), 'B1', EVENTS, GROUPS)
        self.assertIsNone(missing['events']['cycles']['value'])
        self.assertFalse(perf_parse.probe_ok(missing)[0])
        multiplexed = perf_parse.parse(text.replace('10;100.00;;\n', '10;90.00;;\n', 1), 'B1', EVENTS, GROUPS)
        self.assertFalse(perf_parse.probe_ok(multiplexed)[0])
        probe = perf_parse.classify_probe(0, b'', b'', text.encode(), 'B1', EVENTS, GROUPS)
        self.assertTrue(probe['probe_ok'])
        self.assertFalse(perf_parse.classify_probe(1, b'', b'', text.encode(), 'B1', EVENTS, GROUPS)['probe_ok'])
        self.assertTrue(perf_parse.workload_problems(perf_parse.parse(text + '7;;other;10;100.00;;\n', 'B1', EVENTS, GROUPS)))

    def test_bad_and_duplicate_events_fail(self):
        for text in ('20;;instructions;10;100.00;;\n',
                     '20;;instructions;10;100.00;;\n21;;instructions;10;100.00;;\n10;;cycles;10;100.00;;\n',
                     '20;;instructions;10;100.00;;\n-1;;cycles;10;100.00;;\n'):
            with self.assertRaises(perf_parse.ParseError):
                perf_parse.parse(text, 'B1', EVENTS, GROUPS)
        with self.assertRaises(perf_parse.ParseError):
            perf_parse.decode(b'\xff')


if __name__ == '__main__':
    unittest.main()
