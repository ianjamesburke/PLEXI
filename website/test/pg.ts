import { mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import EmbeddedPostgres from 'embedded-postgres';

/**
 * A throwaway PostgreSQL cluster for tests. `embedded-postgres` carries real
 * Postgres server binaries as a devDependency, so tests run the genuine engine
 * without a system install and stay portable to CI. We init a fresh data dir,
 * start on a free port, create a database, and hand back a `DATABASE_URL`.
 */
export interface PgHandle {
  base: string;
  url: string;
  server: EmbeddedPostgres;
}

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.unref();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      if (addr && typeof addr === 'object') {
        const { port } = addr;
        srv.close(() => resolve(port));
      } else {
        srv.close(() => reject(new Error('could not determine a free port')));
      }
    });
  });
}

export async function startPg(): Promise<PgHandle> {
  const base = mkdtempSync(join(tmpdir(), 'plexi-pg-'));
  const port = await freePort();
  const server = new EmbeddedPostgres({
    databaseDir: join(base, 'data'),
    user: 'postgres',
    password: 'postgres',
    port,
    persistent: false,
  });

  await server.initialise();
  await server.start();
  await server.createDatabase('plexi_test');

  const url = `postgresql://postgres:postgres@127.0.0.1:${port}/plexi_test`;
  return { base, url, server };
}

export async function stopPg(handle: PgHandle): Promise<void> {
  try {
    await handle.server.stop();
  } catch {
    // best-effort; the process may already be gone
  }
  rmSync(handle.base, { recursive: true, force: true });
}
