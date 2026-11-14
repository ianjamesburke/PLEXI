#!/usr/bin/env bash
# Print libtest filters, one per line; pass them AFTER cargo test --bin plexi --.
# Taxonomy and scene selection: src/testing/TESTING.md — The Gate.
# Unknown src/ module relationships report ::, never no tests. Batch callers
# must replace that broad fallback with reviewed relevant filters; only the final
# combined candidate runs the full host suite (src/testing/TESTING.md).
# scripts/** selects no Rust filters: the table assigns no evidence block.
# Run script-specific checks separately. Hosted Rust verification is deferred
# until after Alpha landing; test selection does not change hosted CI rules.
# With no arguments, detect branch + staged + unstaged + untracked paths against
# SCOPED_TEST_BASE (default origin/alpha), using the merge base. Set this to the
# actual PR base for aggregate lanes. Detection failure conservatively selects ::.
# The path table lives in scoped-tests.table (tested for drift by test_filter_check.py).
# Selection always exits 0. --selftest is a diagnostic and exits 1 on failure.
# --gate delegates to the sibling docs_gate_scope.py, which owns the rule for
# when a markdown-only diff may skip the gate. Exits 0 for skip, 1 for
# required, and prints the reason.

table_file="$(dirname "$0")/scoped-tests.table"

# Path -> filters comes from scoped-tests.table (first matching row wins).
select_filters() {
    local path patterns filters pattern filter
    local -a pats words
    # A table that cannot be read must never read as "no tests apply": select the broad
    # fallback, which pretests refuse until filters are reviewed by hand.
    if ! grep -q $'\t' "$table_file" 2>/dev/null; then
        printf 'scoped-tests: cannot read a path table with rows at %s; selecting :: (reviewed filters required)\n' "$table_file" >&2
        printf '%s\n' ::
        return 0
    fi
    for path in "$@"; do
        path=${path#./}
        # Markdown is docs only when the classifier positively says skip (exit 0).
        # Binding markdown, a classifier crash or a missing python3 all select ::.
        if [[ "$path" == *.md ]]; then
            if ! python3 "$(dirname "$0")/docs_gate_scope.py" "$path" >/dev/null 2>&1; then
                printf '%s\n' ::
            fi
            continue
        fi
        while IFS=$'\t' read -r patterns filters || [[ -n "$patterns" ]]; do
            [[ -z "$patterns" || "$patterns" == \#* ]] && continue
            IFS='|' read -ra pats <<< "$patterns"
            for pattern in "${pats[@]}"; do
                # shellcheck disable=SC2053 # the table entry is a glob
                if [[ "$path" == $pattern ]]; then
                    read -ra words <<< "$filters"
                    for filter in "${words[@]}"; do printf '%s\n' "$filter"; done
                    break 2
                fi
            done
        done < "$table_file"
    done | LC_ALL=C sort -u
}

selftest() {
    local got first second
    got=$(select_filters notes/guide.md scripts/AGENTS.md)
    [[ -z "$got" ]] || { printf 'FAIL docs-only selected: %s\n' "$got"; return 1; }
    printf 'PASS non-embedded docs-only: <empty>\n'
    got=$(select_filters docs/CONFIG.md)
    [[ "$got" == :: ]] || { printf 'FAIL embedded markdown: %s\n' "$got"; return 1; }
    got=$(select_filters plugins/AGENTS.md)
    [[ "$got" == :: ]] || { printf 'FAIL include_dir markdown: %s\n' "$got"; return 1; }
    printf 'PASS embedded markdown -> :: (code)\n'
    got=$(select_filters src/host/model.rs)
    [[ "$got" == *host::* && "$got" == *testing::* ]] || { printf 'FAIL host: %s\n' "$got"; return 1; }
    printf 'PASS host:\n%s\n' "$got"
    got=$(select_filters src/ui/widgets.rs)
    [[ "$got" == *ui_tests::* && "$got" != *scene_suite* ]] || { printf 'FAIL UI: %s\n' "$got"; return 1; }
    [[ "$(select_filters src/scenes.rs)" == *scene_suite* ]] || { printf 'FAIL shared scene runner requires suite\n'; return 1; }
    [[ "$(select_filters tests/scenes/example.toml)" != *scene_suite* ]] || { printf 'FAIL individual scene selected entire suite\n'; return 1; }
    [[ "$(select_filters src/editor/example.rs)" != *scene_suite* ]] || { printf 'FAIL editor selected entire suite\n'; return 1; }
    printf 'PASS targeted UI and editor; shared scene runner selects suite\n'
    printf 'PASS UI:\n%s\n' "$got"
    first=$(select_filters src/ui/widgets.rs src/host/model.rs src/host/model.rs)
    second=$(select_filters src/ui/widgets.rs src/host/model.rs src/host/model.rs)
    [[ "$first" == "$second" ]] || { printf 'FAIL determinism\n'; return 1; }
    [[ "$first" == "$(select_filters src/host/model.rs src/ui/widgets.rs)" ]] || return 1
    printf 'PASS determinism: byte-identical, sorted and deduplicated\n'
    [[ "$(select_filters src/future/module.rs)" == :: ]] || return 1
    [[ -z "$(select_filters scripts/example.sh plugins/example/app.py sdk/python/example.py)" ]] || return 1
    printf 'PASS unknown src: ::; scripts/PGAP: <empty>\n'
}

if [[ "${1:-}" == --gate ]]; then
    shift
    exec python3 "$(dirname "$0")/docs_gate_scope.py" "$@"
fi
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
