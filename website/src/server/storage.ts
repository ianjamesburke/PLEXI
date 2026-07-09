/**
 * Private artifact storage for PAID apps (stint 0339). Paid `.plexipkg` files
 * live in a private Railway object-storage bucket (S3-compatible), never in the
 * public git repo — the repo is public, so a committed paid artifact is a
 * published one. The bucket credentials are held only by this service; downloads
 * stream through the authenticated gated endpoint, never via signed public URLs
 * (a shareable link would bypass the purchase check).
 *
 * Free artifacts are unaffected: they stay checksum-addressed under
 * website/public/registry/v1/packages/ and are served statically.
 */
import { GetObjectCommand, PutObjectCommand, S3Client } from '@aws-sdk/client-s3';
import { readEnv, requireEnv } from './env';

/** A streamed artifact: a web ReadableStream plus its length and content type. */
export interface ArtifactStream {
  body: ReadableStream;
  contentLength?: number;
  contentType: string;
}

/**
 * The storage backend. Real implementation talks to the bucket; tests override
 * it so they never need live object storage. We own this seam.
 */
export interface ArtifactStore {
  get(key: string): Promise<ArtifactStream | null>;
  put(key: string, body: Uint8Array, contentType?: string): Promise<void>;
}

const ARTIFACT_CONTENT_TYPE = 'application/octet-stream';

let _client: S3Client | undefined;

function bucket(): string {
  return requireEnv('PLEXI_ARTIFACT_BUCKET', 'the private paid-artifact bucket name is required');
}

function s3(): S3Client {
  if (!_client) {
    _client = new S3Client({
      endpoint: requireEnv(
        'PLEXI_ARTIFACT_S3_ENDPOINT',
        'the S3-compatible endpoint for the paid-artifact bucket is required',
      ),
      region: readEnv('PLEXI_ARTIFACT_S3_REGION') ?? 'auto',
      credentials: {
        accessKeyId: requireEnv(
          'PLEXI_ARTIFACT_S3_ACCESS_KEY_ID',
          'the paid-artifact bucket access key id is required',
        ),
        secretAccessKey: requireEnv(
          'PLEXI_ARTIFACT_S3_SECRET_ACCESS_KEY',
          'the paid-artifact bucket secret access key is required',
        ),
      },
      // Railway/MinIO buckets are path-style, not virtual-hosted.
      forcePathStyle: true,
    });
  }
  return _client;
}

const liveStore: ArtifactStore = {
  async get(key: string): Promise<ArtifactStream | null> {
    try {
      const out = await s3().send(new GetObjectCommand({ Bucket: bucket(), Key: key }));
      if (!out.Body) return null;
      // The SDK Body streams lazily; transformToWebStream lets us hand it
      // straight to a Response without buffering the whole artifact in memory.
      const body = (out.Body as { transformToWebStream: () => ReadableStream }).transformToWebStream();
      return {
        body,
        contentLength: out.ContentLength,
        contentType: out.ContentType ?? ARTIFACT_CONTENT_TYPE,
      };
    } catch (err) {
      // A missing object is a null (404 upstream); anything else propagates.
      if (isNotFound(err)) return null;
      console.error(`[storage] get failed key="${key}":`, err);
      throw err;
    }
  },
  async put(key: string, body: Uint8Array, contentType = ARTIFACT_CONTENT_TYPE): Promise<void> {
    try {
      await s3().send(
        new PutObjectCommand({ Bucket: bucket(), Key: key, Body: body, ContentType: contentType }),
      );
      console.info(`[storage] put key="${key}" bytes=${body.byteLength}`);
    } catch (err) {
      console.error(`[storage] put failed key="${key}":`, err);
      throw err;
    }
  },
};

function isNotFound(err: unknown): boolean {
  if (err && typeof err === 'object') {
    const name = (err as { name?: string }).name;
    const status = (err as { $metadata?: { httpStatusCode?: number } }).$metadata?.httpStatusCode;
    return name === 'NoSuchKey' || name === 'NotFound' || status === 404;
  }
  return false;
}

let store: ArtifactStore | null = null;

/** Override the artifact store (tests). */
export function setArtifactStore(s: ArtifactStore): void {
  store = s;
}

/** Restore the live object-storage backend. */
export function resetArtifactStore(): void {
  store = null;
}

/** Fetch a paid artifact stream by object key, or null if it does not exist. */
export function getArtifact(key: string): Promise<ArtifactStream | null> {
  return (store ?? liveStore).get(key);
}

/** Upload a paid artifact by object key (used by the publish flow, stint 0344). */
export function putArtifact(key: string, body: Uint8Array, contentType?: string): Promise<void> {
  return (store ?? liveStore).put(key, body, contentType);
}
