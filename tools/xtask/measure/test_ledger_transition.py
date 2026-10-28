"""Synthetic controls for preserving the ledger and milestone prefix."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ledger_transition


class LedgerTests(unittest.TestCase):
    def setUp(self):
        self.before = {'ledger': 'ledger', 'milestone_criterion': 'criterion',
                       'milestones': [{'id': 'M7', 'commit': 'a' * 40, 'P9': 'preserved'}],
                       'entries': [{'record': 'old', 'commit': 'a' * 40}]}
        self.entry = {'record': 'new', 'subject': 'subject', 'date': '2026-01-01', 'commit': 'b' * 40,
                      'milestone': {'due': True, 'name': 'M8', 'anchor': 'M7'}}
        self.milestone = {'id': 'M8', 'commit': 'b' * 40}

    def test_entry_and_milestone_append_together_and_preserve_history(self):
        original = deepcopy(self.before)
        after, action = ledger_transition.transition(self.before, self.entry, self.milestone)
        self.assertEqual(action, 'appended')
        self.assertEqual(self.before, original)
        self.assertEqual(after['entries'][:-1], self.before['entries'])
        self.assertEqual(after['milestones'][:-1], self.before['milestones'])
        self.assertEqual(ledger_transition.transition(after, self.entry, self.milestone)[1], 'already-present')

    def test_invalid_pair_and_conflicting_duplicate_fail(self):
        for bad in (None, {'id': 'M9', 'commit': 'b' * 40}, {'id': 'M8', 'commit': 'c' * 40}):
            with self.assertRaises(ValueError):
                ledger_transition.transition(self.before, self.entry, bad)
        after, _ = ledger_transition.transition(self.before, self.entry, self.milestone)
        changed = dict(self.entry, subject='different')
        with self.assertRaises(ValueError):
            ledger_transition.transition(after, changed, self.milestone)
        with self.assertRaises(ValueError):
            ledger_transition.transition(self.before, dict(self.entry, unexpected=True), self.milestone)


if __name__ == '__main__':
    unittest.main()
