#!/usr/bin/env python3
"""Offline behavioral checks for PR screenshot publication."""
import base64
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publish_pr_image', Path(__file__).with_name('publish-pr-image.py'))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
HEAD = 'a' * 40
COMMIT = 'b' * 40
BLOB = 'c' * 40
PNG = b'\x89PNG\r\n\x1a\n' + (13).to_bytes(4, 'big') + b'IHDR' + b'\x00' * 16


class Api:
    def __init__(self, private=False):
        self.private = private
        self.calls = []
        self.data = b''
        self.head = HEAD

    def __call__(self, *args, payload=None):
        self.calls.append((args, payload))
        if args[:2] == ('repo', 'view'):
            return {'nameWithOwner': 'owner/repo', 'isPrivate': self.private}
        if args[:2] == ('pr', 'view'):
            return {'state': 'OPEN', 'headRefOid': self.head}
        endpoint = args[-1] if payload is None else args[-3]
        if 'matching-refs/' in endpoint:
            return []
        if endpoint.endswith('/git/refs'):
            return {'ref': payload['ref']}
        if '/contents/' in endpoint and payload is not None:
            self.data = base64.b64decode(payload['content'])
            return {'commit': {'sha': COMMIT}, 'content': {'sha': BLOB}}
        if '/contents/' in endpoint:
            if '?ref=evidence%2Fpr-42' in endpoint:
                raise RuntimeError('gh api failed: HTTP 404')
            return {'sha': BLOB}
        if '/git/blobs/' in endpoint:
            return {'encoding': 'base64', 'content': base64.b64encode(self.data).decode()}
        raise AssertionError(args)


class Response:
    def __enter__(self): return self
    def __exit__(self, *args): pass
    def read(self): return PNG


class PublishTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.file = Path(self.directory.name) / 'capture.png'
        self.file.write_bytes(PNG)
        self.api = Api()
        self.patches = [patch.object(publisher, 'gh', self.api),
                        patch.object(publisher.urllib.request, 'urlopen', return_value=Response())]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)

    def publish(self, **kwargs):
        args = dict(pr=42, image=self.file, source='headless', context='sample scene', expected_head=HEAD, inspected=True)
        args.update(kwargs)
        return publisher.publish(**args)

    def test_public_image_is_read_back_at_immutable_commit_and_labeled(self):
        result = self.publish()
        self.assertIn(f'raw.githubusercontent.com/owner/repo/{COMMIT}/', result)
        self.assertIn('Capture: headless', result)
        self.assertIn(HEAD, result)
        self.assertIn(hashlib.sha256(PNG).hexdigest(), result)
        self.assertTrue(any(payload and payload.get('branch') == 'evidence/pr-42' for _, payload in self.api.calls))

    def test_private_link_is_reviewer_authenticated_and_live_label_is_distinct(self):
        self.api.private = True
        result = self.publish(source='live-host')
        self.assertIn(f'github.com/owner/repo/blob/{COMMIT}/', result)
        self.assertIn('repository access required', result)
        self.assertIn('Capture: live-host', result)
        publisher.urllib.request.urlopen.assert_not_called()

    def test_uninspected_or_non_png_refused_before_network(self):
        with self.assertRaisesRegex(ValueError, 'Inspect'):
            self.publish(inspected=False)
        self.file.write_bytes(b'not a PNG')
        with self.assertRaisesRegex(ValueError, 'PNG'):
            self.publish()
        self.assertEqual(self.api.calls, [])

    def test_head_mismatch_fails_before_creating_branch(self):
        self.api.head = 'd' * 40
        with self.assertRaisesRegex(ValueError, 'current head'):
            self.publish()
        self.assertEqual(len(self.api.calls), 2)

    def test_failed_public_readback_is_not_evidence(self):
        publisher.urllib.request.urlopen.return_value.read = lambda: b'incorrect'
        with self.assertRaisesRegex(RuntimeError, 'readback did not match'):
            self.publish()

    def test_head_movement_during_upload_fails(self):
        original = self.api.__call__
        count = 0
        def moved(*args, **kwargs):
            nonlocal count
            if args[:2] == ('pr', 'view'):
                count += 1
                if count == 2:
                    self.api.head = 'd' * 40
            return original(*args, **kwargs)
        with patch.object(publisher, 'gh', moved):
            with self.assertRaisesRegex(RuntimeError, 'head changed'):
                self.publish()


if __name__ == '__main__':
    unittest.main()
