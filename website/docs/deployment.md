# Deployment

## Deploying to Railway

The site builds via `Dockerfile` and runs as a Node standalone server on Railway.
Subscribers persist in a SQLite file on a Railway volume mounted at `/data`.

One-time setup:

1. **Init and link the project.** From the repo root:
   ```bash
   railway init
   # or, if the project already exists:
   railway link
   ```
2. **Attach a persistent volume.** Railway's `railway.json` schema does not
   currently support declaring volumes, so this is a manual step in the
   dashboard:
   - Service → Settings → Volumes → **New Volume**
   - Size: **1 GB** (more than enough for an email list)
   - Mount path: **`/data`**
3. **Set the environment variable** (Service → Variables):
   ```
   DB_PATH=/data/subscribers.db
   ```
   The Dockerfile already defaults `DB_PATH` to this value, but setting it
   explicitly in Railway makes the wiring obvious and survives Dockerfile
   changes.
4. **Deploy.** Either:
   ```bash
   railway up
   ```
   …or push to the connected GitHub branch and let the Railway integration
   build.

## Pulling the subscriber list

SSH into the running service and dump the table to CSV:

```bash
railway ssh
sqlite3 /data/subscribers.db ".headers on" ".mode csv" "SELECT * FROM subscribers;" > subscribers.csv
exit
```

Then copy it down with `railway run` or just `cat` it inside the SSH session
and paste — whichever is faster for the size of the list.

## Local dev

```bash
npm install
npm run dev
```

The DB defaults to `./data/subscribers.db` (created on first request). The
`data/` directory is gitignored.

To inspect locally:

```bash
sqlite3 ./data/subscribers.db "SELECT * FROM subscribers;"
```
