# Drafts

Versioned working copies of blog posts. One directory per article, one file per version.

## Structure

```
drafts/
  _template/v1.md       ← copy this to start a new article
  your-article-slug/
    v1.md               ← human draft
    v2.md               ← AI proofread
    v3.md               ← human revision
    ...
```

## Workflow

**Start a new article:**
```sh
scripts/new-draft <slug>
# creates drafts/<slug>/v1.md from the template
```

**After you've written v1, get an AI proofread:**
```sh
scripts/proofread <slug>
# reads latest version, writes the next version
# (run this via Claude Code: /proofread <slug>)
```

**Publish a version to the blog:**
```sh
scripts/publish-draft <slug> [version]
# strips _version/_type/_notes frontmatter
# writes to src/content/blog/YYYY-MM-DD-<slug>.md
# defaults to latest version if no version specified
```

## Version frontmatter

Each version file has standard blog frontmatter plus `_`-prefixed version fields:

| Field | Description |
|---|---|
| `_version` | Integer, increments with each pass |
| `_type` | `human-draft`, `ai-proofread`, or `human-revision` |
| `_based_on` | Version number this was derived from (`~` for original) |
| `_notes` | Short note on what changed in this pass |

These fields are stripped by `publish-draft` and never appear in the live blog.
