#!/usr/bin/env bash
# Print libtest filters, one per line; pass them AFTER cargo test --bin plexi --.
# Taxonomy: .agents/skills/testing/SKILL.md, Step 1 — Classify the Diff.
# Unknown src/ module relationships report ::, never no tests. Batch callers
# must replace that broad fallback with reviewed relevant filters; only the final
# combined candidate runs the full host suite (src/testing/TESTING.md).
# scripts/** selects no Rust filters: the table assigns no evidence block.
# Run script-specific checks separately. Hosted Rust verification is deferred
# until after Alpha landing; test selection does not change hosted CI rules.
# With no arguments, detect branch + staged + unstaged + untracked paths against
# SCOPED_TEST_BASE (default origin/alpha), using the merge base. Set this to the
# actual PR base for aggregate lanes. Detection failure conservatively selects ::.
# Selection always exits 0. --selftest is a diagnostic and exits 1 on failure.

select_filters() {
    local path
    for path in "$@"; do
        path=${path#./}
        case "$path" in
            *.md|docs/*|.agents/*|.claude/*) ;;
            src/host/*) printf '%s\n' host:: testing:: ui_tests:: ;;
            src/ui/*|src/render/*|src/ui_tests.rs|src/scenes.rs|tests/scenes/*)
                printf '%s\n' ui:: render:: ui_tests:: testing:: scene_suite ;;
            src/cli/*) printf '%s\n' cli:: cli_tests:: testing:: ;;
            src/editor/*|src/app/text_editor_app.rs)
                printf '%s\n' editor:: editor_gate ui_tests:: scene_suite ;;
            src/config/*|src/workspace/*)
                printf '%s\n' config:: workspace:: testing:: ;;
            src/*) printf '%s\n' :: ;;
            apps/*|sdk/python/*) ;; # PGAP evidence is separate, not libtest.
            scripts/*) ;; # Deliberate decision documented above.
            *) ;; # docs/skills/config-only: no host evidence per classification.
        esac
    done | LC_ALL=C sort -u
}

selftest() {
    local got first second
    got=$(select_filters docs/CONFIG.md AGENTS.md .agents/skills/testing/SKILL.md)
    [[ -z "$got" ]] || { printf 'FAIL docs-only selected: %s\n' "$got"; return 1; }
    printf 'PASS docs-only: <empty>\n'
    got=$(select_filters src/host/model.rs)
    [[ "$got" == *host::* && "$got" == *testing::* ]] || { printf 'FAIL host: %s\n' "$got"; return 1; }
    printf 'PASS host:\n%s\n' "$got"
    got=$(select_filters src/ui/widgets.rs)
    [[ "$got" == *ui_tests::* && "$got" == *scene_suite* ]] || { printf 'FAIL UI: %s\n' "$got"; return 1; }
    printf 'PASS UI:\n%s\n' "$got"
    first=$(select_filters src/ui/widgets.rs src/host/model.rs src/host/model.rs)
    second=$(select_filters src/ui/widgets.rs src/host/model.rs src/host/model.rs)
    [[ "$first" == "$second" ]] || { printf 'FAIL determinism\n'; return 1; }
    [[ "$first" == "$(select_filters src/host/model.rs src/ui/widgets.rs)" ]] || return 1
    printf 'PASS determinism: byte-identical, sorted and deduplicated\n'
    [[ "$(select_filters src/future/module.rs)" == :: ]] || return 1
    [[ -z "$(select_filters scripts/example.sh apps/example/app.py sdk/python/example.py)" ]] || return 1
    printf 'PASS unknown src: ::; scripts/PGAP: <empty>\n'
}

if [[ "${1:-}" == --selftest ]]; then
    selftest
    exit $?
fi
if [[ $# -gt 0 ]]; then
    select_filters "$@"
else
    root=$(git rev-parse --show-toplevel) &&
        base=$(git merge-base "${SCOPED_TEST_BASE:-origin/alpha}" HEAD)
    if [[ $? -ne 0 ]]; then
        printf 'scoped-tests: cannot detect base; selecting all test paths\n' >&2
        printf '%s\n' ::
    else
        paths=()
        # NUL delimiters preserve spaces, Unicode, newlines and Git-quoted names.
        # A failed producer emits an unknown src path, forcing the broad fallback.
        while IFS= read -r -d '' path; do
            paths+=("$path")
        done < <(
            git -C "$root" diff --name-only --no-renames -z "$base" -- ||
                printf 'src/__detection_failed__\0'
            git -C "$root" ls-files --others --exclude-standard -z ||
                printf 'src/__detection_failed__\0'
        )
        select_filters "${paths[@]}"
    fi

fi
exit 0
