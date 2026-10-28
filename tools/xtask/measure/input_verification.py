"""Regenerate measurement inputs from their recorded seed and compare bytes."""
import hashlib
import json
from pathlib import Path
import tempfile

import gen_inputs
import input_closure


def verify(root):
    root = Path(root).resolve(strict=True)
    data = (root / 'MANIFEST.json').read_bytes()
    manifest = json.loads(data)
    if not isinstance(manifest, dict) or type(manifest.get('seed')) is not int or manifest['seed'] < 0:
        raise ValueError('input manifest requires a nonnegative integer seed')
    with tempfile.TemporaryDirectory(prefix='rune-input-check-') as tmp:
        generated = Path(tmp) / 'generated'
        gen_inputs.main(str(generated), manifest['seed'])
        reference = json.loads((generated / 'MANIFEST.json').read_bytes())
        if reference != manifest:
            raise ValueError('input manifest differs from seed and model')
        paths = ['MANIFEST.json']
        for stream in reference['streams'].values():
            name = stream['path']
            input_closure._relative(name)
            observed = input_closure._pin(root, name)
            if observed['sha256'] != stream['sha256']:
                raise ValueError('input stream differs from seed')
            paths.append(name)
        pins = input_closure.create(root, paths)
        spec_pin = next(item for item in pins['files'] if item['path'] == 'MANIFEST.json')
        if hashlib.sha256(data).hexdigest() != spec_pin['sha256']:
            raise ValueError('input manifest changed during verification')
        return {'seed': manifest['seed'], 'streams': len(reference['streams']), 'closure': pins}
