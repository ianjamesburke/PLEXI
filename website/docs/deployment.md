# Deployment

## Deploy pipeline (stint 0345)

Production deploys from the **`alpha` branch** via the Railway GitHub
integration — every merged website change ships automatically. The website is
not the app binary (downloads redirect to GitHub releases), so it does not
wait on `main` release promotions. Changed 2026-07-02; before that the
service tracked `main` and prod drifted weeks behind alpha.

- Manual deploy of the current tree: `just website-deploy` (repo root).
- Post-deploy verification: `just website-smoke` — checks the home/download
  pages, the install redirect, the registry index, and downloads every
  registry artifact to verify its checksum. Run it after every deploy;
  nonzero exit means production is broken.

## Deploying to Railway

The site builds via `Dockerfile` and runs as a Node standalone server on
Railway. Persistence is **Railway Postgres**; the schema and its tables are
described in `docs/marketplace-monetization.md` (repo root). Migrations in
`src/server/migrations/` run automatically on first request.

One-time setup:

1. **Init and link the project.** From the repo root:
   ```bash
   railway init
   # or, if the project already exists:
   railway link
   ```
2. **Add a Postgres database** (Railway dashboard → New → Database → PostgreSQL).
   Railway injects a `DATABASE_URL` reference variable into the web service —
   confirm the web service has `DATABASE_URL` under Variables.
3. **Set the remaining service variables** (Service → Variables):
   ```
   DATABASE_URL=<injected by the Postgres plugin>
   RESEND_API_KEY=<your Resend key>
   PUBLIC_SITE_URL=https://plexiapp.com
   ```
   All three are required — `DATABASE_URL` and `RESEND_API_KEY` fail the service
   at boot, and `PUBLIC_SITE_URL` throws when a magic-link URL is first built
   (no silent fallback).
4. **Deploy.** Push to the connected GitHub branch, or `railway up`.

## Backups (user action)

**Enable automated backups in the Railway dashboard** — this is a manual step:
Postgres service → **Backups** → enable scheduled daily backups and set a
retention window. Do this once after provisioning the database.

Restore procedure (run locally with the service's `DATABASE_URL`):

```bash
# Snapshot the live database to a local file
pg_dump "$DATABASE_URL" -Fc -f plexi-backup.dump

# Restore a snapshot into a target database (drops/recreates objects)
pg_restore --clean --if-exists -d "$DATABASE_URL" plexi-backup.dump
```

`-Fc` is Postgres's custom compressed format, which `pg_restore` consumes.
For a plain-SQL dump use `pg_dump "$DATABASE_URL" -f plexi-backup.sql` and
restore with `psql "$DATABASE_URL" -f plexi-backup.sql`.

## Inspecting data

```bash
railway connect Postgres   # opens psql against the service database
# then, e.g.
SELECT email, created_at FROM subscribers ORDER BY created_at DESC;
```

## Local dev

```bash
npm install
npm run dev
```

Set `DATABASE_URL` to a local Postgres instance (and `RESEND_API_KEY`,
`PUBLIC_SITE_URL`) before starting the dev server. Tests boot their own
throwaway Postgres cluster and need no external database.
