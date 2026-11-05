#!/usr/bin/env python3
"""Publish an inspected PR screenshot on a retained evidence branch."""
import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote


def gh(*args, payload=None):
    command = ['gh', *args]
    result = subprocess.run(command, input=None if payload is None else json.dumps(payload),
                            text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"{' '.join(command)} failed: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{' '.join(command)} returned invalid JSON") from exc


def publish(pr, image, source, context, expected_head, inspected):
    if not inspected:
        raise ValueError('Inspect the image and confirm it is safe to publish with --inspected')
    data = Path(image).read_bytes()
    if len(data) < 24 or data[:8] != b'\x89PNG\r\n\x1a\n' or data[12:16] != b'IHDR':
        raise ValueError('image must be a nonempty PNG')
    if not context.strip() or '\n' in context or '\r' in context:
        raise ValueError('context must be a nonempty single line')
    if not re.fullmatch(r'[0-9a-f]{40}', expected_head):
        raise ValueError('head must be a full lowercase 40-character commit SHA')
    repo = gh('repo', 'view', '--json', 'nameWithOwner,isPrivate')
    name = repo['nameWithOwner']
    review = gh('pr', 'view', str(pr), '--json', 'state,headRefOid')
    if review['state'] != 'OPEN' or review['headRefOid'] != expected_head:
        raise ValueError('PR must be open and its current head must match --head')
    branch = f'evidence/pr-{pr}'
    # Branch creation is append-only and each PR gets its own retained ref.
    refs = gh('api', f'repos/{name}/git/matching-refs/heads/{branch}')
    if not any(ref.get('ref') == f'refs/heads/{branch}' for ref in refs):
        gh('api', '--method', 'POST', f'repos/{name}/git/refs', '--input', '-',
           payload={'ref': f'refs/heads/{branch}', 'sha': expected_head})
    digest = hashlib.sha256(data).hexdigest()
    path = f'pr-evidence/{expected_head}/{digest}.png'
    # The API creates nested directories; probe for an existing exact path to
    # avoid overwrites, including retries after an interrupted upload.
    try:
        existing = gh('api', f'repos/{name}/contents/{path}?ref={quote(branch, safe="")}')
    except RuntimeError as exc:
        if 'HTTP 404' not in str(exc):
            raise
        existing = None
    if existing is None:
        uploaded = gh('api', '--method', 'PUT', f'repos/{name}/contents/{path}', '--input', '-',
                      payload={'message': f'Add visual evidence for PR #{pr} at {expected_head[:12]}',
                               'content': base64.b64encode(data).decode(), 'branch': branch})
        evidence_commit = uploaded['commit']['sha']
        blob = uploaded['content']['sha']
    else:
        ref = gh('api', f'repos/{name}/git/ref/heads/{branch}')
        evidence_commit = ref['object']['sha']
        blob = existing['sha']
    result = gh('api', f'repos/{name}/git/blobs/{blob}')
    if result.get('encoding') != 'base64' or hashlib.sha256(base64.b64decode(result['content'])).hexdigest() != digest:
        raise RuntimeError('GitHub blob readback did not match the uploaded image')
    # A fetch at the immutable commit confirms that the ref/commit actually
    # contains the expected blob, not merely that an unattached blob exists.
    committed = gh('api', f'repos/{name}/contents/{path}?ref={evidence_commit}')
    if committed['sha'] != blob:
        raise RuntimeError('evidence commit does not contain the uploaded blob')
    review = gh('pr', 'view', str(pr), '--json', 'state,headRefOid')
    if review['state'] != 'OPEN' or review['headRefOid'] != expected_head:
        raise RuntimeError('PR head changed during upload; capture and publish evidence for the new head')
    safe_path = quote(path, safe='/')
    if repo['isPrivate']:
        url = f'https://github.com/{name}/blob/{evidence_commit}/{safe_path}'
        link = f'[View screenshot (repository access required)]({url})'
    else:
        url = f'https://raw.githubusercontent.com/{name}/{evidence_commit}/{safe_path}'
        # Verify the same URL a public reviewer will load, not just the API.
        try:
            with urllib.request.urlopen(url, timeout=30) as response:
                if hashlib.sha256(response.read()).hexdigest() != digest:
                    raise RuntimeError('public screenshot readback did not match the uploaded image')
        except urllib.error.URLError as exc:
            raise RuntimeError(f'public screenshot URL could not be read: {exc}') from exc
        link = f'![{source} screenshot]({url})'
    return f'{link}\n\nCapture: {source}; {context}. PR head: `{expected_head}`. Evidence commit: `{evidence_commit}`. SHA-256: `{digest}`.'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pr', type=int, required=True)
    parser.add_argument('--image', required=True)
    parser.add_argument('--source', choices=['headless', 'live-host'], required=True)
    parser.add_argument('--context', required=True, help='feature, scene, and capture details')
    parser.add_argument('--head', required=True, help='full PR head SHA captured')
    parser.add_argument('--inspected', action='store_true', help='image inspected and safe for repository visibility')
    args = parser.parse_args()
    if args.pr <= 0:
        parser.error('--pr must be positive')
    try:
        print(publish(args.pr, args.image, args.source, args.context, args.head, args.inspected))
    except (ValueError, OSError, RuntimeError, KeyError) as exc:
        print(f'visual evidence: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
