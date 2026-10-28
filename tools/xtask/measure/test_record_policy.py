"""Record coverage controls use synthetic fresh seal evidence."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import record_policy


class RecordPolicyTests(unittest.TestCase):
    def setUp(self):
        self.commit = 'a' * 40
        def proof(protocol, groups):
            roles = {}
            for role, profile in groups:
                for native in (False, True):
                    key = f'{role}-{profile}-{native}'
                    roles[key] = {'role': role, 'profile': profile, 'native': native,
                                  'commit': {'candidate': self.commit, 'tip': 'b' * 40,
                                             'base': 'c' * 40, 'milestone': 'd' * 40}[role]}
            return {'status': 'PASS', 'protocol': protocol, 'candidate_commit': self.commit,
                    'roles': roles, 'assessment': {'cycle_flags': []}}
        self.proofs = [proof('callgrind-v1', [(role, profile) for profile, roles in
                       [('cgu1', ('base', 'tip', 'candidate')), ('cgu16', ('milestone', 'tip', 'candidate'))] for role in roles])]
        self.proofs += [proof('counter2-v1', [(role, profile) for role in ('base', 'tip', 'candidate')]) for profile in ('cgu1', 'cgu16')]
        self.proofs += [proof('k1-v1', [(role, profile) for role in (('tip', 'candidate') if profile == 'cgu1' else ('tip', 'candidate', 'milestone'))]) for profile in ('cgu1', 'cgu16')]

    def test_complete_coverage_preserves_tip_and_milestone(self):
        result = record_policy.validate(self.proofs, self.commit)
        self.assertEqual(result['tip_commit'], 'b' * 40)
        self.assertEqual(result['milestone_commit'], 'd' * 40)
        self.assertEqual(len(result['protocols']), 5)

    def test_missing_duplicate_and_mismatched_roles_fail(self):
        with self.assertRaisesRegex(ValueError, 'five'):
            record_policy.validate(self.proofs[:-1], self.commit)
        duplicate = deepcopy(self.proofs)
        duplicate[-1] = deepcopy(duplicate[-2])
        with self.assertRaises(ValueError):
            record_policy.validate(duplicate, self.commit)
        drift = deepcopy(self.proofs)
        for role in drift[-1]['roles'].values():
            if role['role'] == 'tip':
                role['commit'] = 'e' * 40
        with self.assertRaisesRegex(ValueError, 'coverage differs'):
            record_policy.validate(drift, self.commit)


if __name__ == '__main__':
    unittest.main()
