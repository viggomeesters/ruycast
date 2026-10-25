import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('clipboard_import', Path(__file__).resolve().parents[1] / 'scripts/import-clipboard.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ClipboardImportTests(unittest.TestCase):
    def row(self, **changes):
        return dict(category='text', text='Synthetic content', applicationPath='/Applications/Fixture.app', createdAt='2026-10-25T00:00:00Z', **changes)

    def normalize(self, rows, **options):
        return module.normalize({'builtin_package_clipboardHistory': {'clipboardHistoryRecords': rows}}, **options)

    def test_text_stable_identity_dedup_and_date(self):
        row = self.row()
        clips, counts = self.normalize([row, row])
        self.assertEqual(len(clips), 1)
        self.assertEqual(clips, self.normalize([row])[0])
        self.assertEqual(counts['duplicate'], 1)
        self.assertEqual(clips[0]['date'], '2026-10-25T00:00:00Z')

    def test_files_are_references_and_missing_images_explicit(self):
        file = self.row(); file.update(category='file', filePath='/nonexistent/synthetic file.txt')
        image = self.row(); image.update(category='image', filePath='/nonexistent/synthetic.png')
        clips, counts = self.normalize([file, image])
        self.assertEqual(clips[0]['fileURLs'], ['file:///nonexistent/synthetic%20file.txt'])
        self.assertEqual(counts['missingImageAssets'], 1)
        self.assertEqual(counts['files'], 1)

    def test_malformed_oversize_and_unsupported(self):
        invalid = self.row(); invalid['createdAt'] = 'bad'
        unsupported = self.row(); unsupported['category'] = 'encrypted'
        clips, counts = self.normalize([invalid, unsupported, self.row()], max_text_bytes=1)
        self.assertEqual(clips, [])
        self.assertEqual((counts['invalid'], counts['unsupported'], counts['oversized']), (1, 1, 1))

    def test_bounded_batches_and_retry_identity(self):
        items = [{'id': str(i), 'text': 'a' * 20000} for i in range(151)]
        chunks = list(module.batches(items, '0' * 64))
        self.assertGreater(len(chunks), 2)
        self.assertEqual(chunks, list(module.batches(items, '0' * 64)))
        self.assertTrue(all(len(chunk['items']) <= 100 and len(module.json.dumps(chunk).encode()) < module.MAX_WIRE for chunk in chunks))
        self.assertEqual(sum(len(chunk['items']) for chunk in chunks), 151)


if __name__ == '__main__':
    unittest.main()
