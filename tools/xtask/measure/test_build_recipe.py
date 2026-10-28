"""Controls for profile rendering against a temporary verified source."""
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_recipe
import source_export


class RecipeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'repo'
        (self.repo / 'crates' / 'rune').mkdir(parents=True)
        source_export._git(self.repo, 'init', '-q')
        source_export._git(self.repo, 'config', 'user.name', 'Test')
        source_export._git(self.repo, 'config', 'user.email', 'test@example.invalid')
        (self.repo / 'crates' / 'rune' / 'Cargo.toml').write_text('[package]\nname="rune"\nversion="0.0.0"\n[features]\nexecution=[]\nnative-code=[]\n')
        source_export._git(self.repo, 'add', 'crates/rune/Cargo.toml')
        source_export._git(self.repo, 'commit', '-qm', 'fixture')
        self.export = source_export.export(self.repo, 'HEAD', self.root / 'export')

    def test_profiles_preserve_common_recipe(self):
        manifests = []
        for profile in ('cgu1', 'cgu16'):
            out = self.root / profile
            receipt = build_recipe.prepare(self.export['source'], self.export['commit'], profile, out)
            self.assertEqual(receipt['profile'], profile)
            manifests.append(tomllib.loads((out / 'Cargo.toml').read_text()))
            self.assertEqual((out / 'Cargo.lock').read_bytes(), (build_recipe.HARNESS / 'Cargo.lock').read_bytes())
        self.assertEqual([m['profile']['release']['codegen-units'] for m in manifests], [1, 16])
        for manifest in manifests:
            self.assertEqual(manifest['dependencies']['rune']['path'], self.export['source'] + '/crates/rune')
            self.assertEqual(manifest['package']['name'], 'p2-probe-harness')
            self.assertFalse(manifest['profile']['release']['lto'])
        out = self.root / 'aliased'
        build_recipe.prepare(self.export['source'], self.export['commit'], 'cgu1', out, source_alias='/tmp/rune-build/source')
        self.assertEqual(tomllib.loads((out / 'Cargo.toml').read_text())['dependencies']['rune']['path'], '/tmp/rune-build/source/crates/rune')

    def test_legacy_features_are_selected_from_source(self):
        manifest = self.repo / 'crates/rune/Cargo.toml'
        manifest.write_text('[package]\nname="rune"\nversion="0.0.0"\n[features]\nfresh-core=[]\nfresh-core-native=[]\n')
        source_export._git(self.repo, 'add', 'crates/rune/Cargo.toml')
        source_export._git(self.repo, 'commit', '-qm', 'legacy')
        exported = source_export.export(self.repo, 'HEAD', self.root / 'legacy-export')
        out = self.root / 'legacy-recipe'
        receipt = build_recipe.prepare(exported['source'], exported['commit'], 'cgu1', out)
        rendered = tomllib.loads((out / 'Cargo.toml').read_text())
        self.assertEqual(receipt['native_feature'], 'fresh-core-native')
        self.assertEqual(rendered['dependencies']['rune']['features'], ['fresh-core'])
        self.assertEqual(rendered['features']['native'], ['rune/fresh-core-native'])
        manifest = Path(exported['source']) / 'crates/rune/Cargo.toml'
        manifest.write_text('[features]\nexecution=[]\nnative-code=[]\nfresh-core=[]\nfresh-core-native=[]\n')
        with self.assertRaisesRegex(ValueError, 'exactly one'):
            build_recipe.feature_names(exported['source'])

    def test_dirty_source_and_unknown_profile_are_rejected(self):
        with self.assertRaises(ValueError):
            build_recipe.prepare(self.export['source'], self.export['commit'], 'other', self.root / 'bad')
        with self.assertRaises(ValueError):
            build_recipe.prepare(self.export['source'], self.export['commit'], 'cgu1', Path(self.export['source']) / 'bad')
        with self.assertRaises(ValueError):
            build_recipe.prepare(self.export['source'], self.export['commit'], 'cgu1', self.root / 'bad', source_alias='/source/../other')
        (Path(self.export['source']) / 'crates/rune/Cargo.toml').write_text('changed')
        with self.assertRaises(ValueError):
            build_recipe.prepare(self.export['source'], self.export['commit'], 'cgu1', self.root / 'bad')


if __name__ == '__main__':
    unittest.main()
