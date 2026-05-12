import Database from 'better-sqlite3';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

const DB_PATH = process.env.DB_PATH ?? './data/subscribers.db';

// Ensure the parent directory exists before SQLite tries to open the file.
try {
  mkdirSync(dirname(DB_PATH), { recursive: true });
} catch (err) {
  console.error(`[db] failed to create db directory for ${DB_PATH}:`, err);
  throw err;
}

let _db: Database.Database;
try {
  _db = new Database(DB_PATH);
  _db.pragma('journal_mode = WAL');
  _db.exec(`
    CREATE TABLE IF NOT EXISTS subscribers (
      email TEXT PRIMARY KEY,
      created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
      source TEXT
    );
  `);
} catch (err) {
  console.error(`[db] failed to open or initialize sqlite at ${DB_PATH}:`, err);
  throw err;
}

export const db = _db;

const insertStmt = db.prepare(
  'INSERT INTO subscribers (email, source) VALUES (?, ?) ON CONFLICT(email) DO NOTHING',
);

export function addSubscriber(email: string, source?: string): void {
  try {
    insertStmt.run(email, source ?? null);
  } catch (err) {
    console.error(
      `[db] addSubscriber failed for email="${email}" source="${source ?? ''}":`,
      err,
    );
    throw err;
  }
}
