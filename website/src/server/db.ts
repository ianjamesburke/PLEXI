import pg from 'pg';
import { readEnv } from './env';

/** Thrown at boot when DATABASE_URL is absent — fail fast, no silent fallback. */
export class MissingDatabaseUrlError extends Error {
  constructor() {
    super('DATABASE_URL is not set — the account service requires a Postgres connection string');
    this.name = 'MissingDatabaseUrlError';
  }
}

function connectionString(): string {
  const url = readEnv('DATABASE_URL');
  if (!url) throw new MissingDatabaseUrlError();
  return url;
}

let _pool: pg.Pool | undefined;

/** The shared connection pool, created lazily on first use. */
export function getPool(): pg.Pool {
  if (!_pool) {
    _pool = new pg.Pool({ connectionString: connectionString() });
    _pool.on('error', (err) => {
      console.error('[db] idle client error:', err);
    });
  }
  return _pool;
}

/** Close the pool (tests / graceful shutdown). Safe to call when never opened. */
export async function closePool(): Promise<void> {
  if (_pool) {
    const p = _pool;
    _pool = undefined;
    await p.end();
  }
}

// Migration SQL is bundled at build time via Vite's glob import so the .sql
// files travel into dist/ and are available to both Astro and vitest.
const migrationSources = import.meta.glob('./migrations/*.sql', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

let _migrated: Promise<void> | undefined;

/** Run pending migrations exactly once per process. Idempotent and memoized. */
export function ensureMigrated(): Promise<void> {
  if (!_migrated) {
    _migrated = runMigrations().catch((err) => {
      // Reset so a later call can retry after a transient failure.
      _migrated = undefined;
      throw err;
    });
  }
  return _migrated;
}

/**
 * Session-wide advisory-lock key so concurrent replicas serialize migrations.
 * Held for the migration transaction only (xact lock auto-releases on COMMIT).
 */
const MIGRATION_ADVISORY_LOCK_KEY = 727374;

async function runMigrations(): Promise<void> {
  const pool = getPool();
  const client = await pool.connect();
  try {
    await client.query('BEGIN');
    // Serialize against other replicas before touching schema_migrations.
    await client.query('SELECT pg_advisory_xact_lock($1)', [MIGRATION_ADVISORY_LOCK_KEY]);
    await client.query(`
      CREATE TABLE IF NOT EXISTS schema_migrations (
        filename   TEXT PRIMARY KEY,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
      );
    `);

    const paths = Object.keys(migrationSources).sort();
    for (const path of paths) {
      const filename = path.replace(/^.*\//, '');
      const sql = migrationSources[path];
      const already = await client.query('SELECT 1 FROM schema_migrations WHERE filename = $1', [
        filename,
      ]);
      if (already.rowCount === 0) {
        await client.query(sql);
        await client.query('INSERT INTO schema_migrations (filename) VALUES ($1)', [filename]);
        console.info(`[db] applied migration ${filename}`);
      }
    }
    await client.query('COMMIT');
  } catch (err) {
    await client.query('ROLLBACK');
    console.error('[db] migrations failed:', err);
    throw err;
  } finally {
    client.release();
  }
}

/**
 * Migration-gated query. Awaits {@link ensureMigrated} then runs on the pool,
 * so callers never repeat the gate. Use for single statements.
 */
export async function query<R extends pg.QueryResultRow = pg.QueryResultRow>(
  text: string,
  params?: unknown[],
): Promise<pg.QueryResult<R>> {
  await ensureMigrated();
  return getPool().query<R>(text, params);
}

/**
 * Migration-gated client checkout for multi-statement transactions. Awaits
 * {@link ensureMigrated} then leases a client. Caller must `release()`.
 */
export async function getClient(): Promise<pg.PoolClient> {
  await ensureMigrated();
  return getPool().connect();
}

/** Insert a newsletter subscriber (idempotent on email). */
export async function addSubscriber(email: string, source?: string): Promise<void> {
  try {
    await query(
      `INSERT INTO subscribers (email, source) VALUES ($1, $2)
       ON CONFLICT (email) DO NOTHING`,
      [email, source ?? null],
    );
  } catch (err) {
    console.error(
      `[db] addSubscriber failed for email="${email}" source="${source ?? ''}":`,
      err,
    );
    throw err;
  }
}
