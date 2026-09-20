# Pending workflows

`build-windows.yml` belongs at `.github/workflows/build-windows.yml`. It is
staged here only because the token that pushed this branch lacks GitHub's
`workflow` OAuth scope, which is required to create or modify any file under
`.github/workflows/`.

To activate it:

```sh
gh auth refresh -s workflow          # one-time, opens a browser
git mv .github/workflows-pending/build-windows.yml .github/workflows/
git rm .github/workflows-pending/README.md
git commit -m "ci(windows): compile plexi.exe on windows-latest"
git push
```

Then run it from the Actions tab (it has a `workflow_dispatch` trigger), or
just push to this branch — the `push` trigger matches `feature/windows-**`.
