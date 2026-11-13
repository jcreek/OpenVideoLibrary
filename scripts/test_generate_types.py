import importlib.util
import json
from pathlib import Path
import unittest

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('generate_types', root / 'scripts/generate-types.py')
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class ContractGenerationTests(unittest.TestCase):
    def test_audio_limit_nullability_uses_openapi_31_and_generated_client(self):
        contract = json.loads((root / 'docs/openapi.json').read_text())
        self.assertTrue(contract['openapi'].startswith('3.1.'))
        capabilities = contract['components']['schemas']['ClientCapabilities']
        limit = capabilities['properties']['max_audio_channels']
        self.assertEqual(set(limit['type']), {'integer', 'null'})
        self.assertNotIn('nullable', limit)
        self.assertEqual((limit['minimum'], limit['maximum']), (1, 8))
        self.assertNotIn('max_audio_channels', capabilities['required'])
        self.assertEqual(generator.ts(limit), '(number | null)')
        generated = (root / 'web/src/api.generated.ts').read_text()
        self.assertIn('export type ClientCapabilities = ' + generator.ts(capabilities) + ';', generated)

    def test_openapi_31_union_inside_dictionary_and_array(self):
        schema = {'type': 'object', 'additionalProperties': {'type': 'array', 'items': {'type': ['string', 'null']}}}
        self.assertEqual(generator.ts(schema), 'Record<string, ((string | null))[]>')


if __name__ == '__main__':
    unittest.main()
