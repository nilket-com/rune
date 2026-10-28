"""Validate the committed source-only milestone registry."""
import json
from pathlib import Path
import re

REGISTRY = Path(__file__).resolve().parent / 'milestones.json'


def load(path=None):
    registry = json.loads(Path(path if path is not None else REGISTRY).read_bytes())
    if (not isinstance(registry, dict) or set(registry) != {'schema', 'current', 'milestones'}
            or registry['schema'] != 1 or not isinstance(registry['milestones'], list)
            or not registry['milestones']):
        raise ValueError('invalid milestone registry')
    previous = -1
    for item in registry['milestones']:
        if (not isinstance(item, dict) or set(item) != {'id', 'commit'}
                or not isinstance(item['id'], str) or not re.fullmatch(r'M(?:0|[1-9][0-9]*)', item['id'])
                or not isinstance(item['commit'], str) or not re.fullmatch('[0-9a-f]{40}', item['commit'])):
            raise ValueError('invalid milestone source identity')
        number = int(item['id'][1:])
        if number <= previous:
            raise ValueError('milestone identities must be ordered and unique')
        previous = number
    if registry['current'] != registry['milestones'][-1]['id']:
        raise ValueError('current milestone must be the latest registered source')
    return registry
