PYTHON_VERSION := "3.12.13"
PYTHON_PBS_DATE := "20260414"

export RUSTFLAGS := "-D warnings"

# Download the python-build-standalone runtime into assets/python/ for bundling.
# Skips if the correct version is already present. macOS only.
fetch-python-runtime:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "$(uname)" != "Darwin" ]]; then
        echo "fetch-python-runtime: macOS only, skipping"
        exit 0
    fi

    ARCH=$(uname -m)
    if [[ "$ARCH" == "arm64" ]]; then
        PBS_ARCH="aarch64-apple-darwin"
    else
        PBS_ARCH="x86_64-apple-darwin"
    fi

    VERSION="{{PYTHON_VERSION}}"
    DATE="{{PYTHON_PBS_DATE}}"
    EXPECTED="${VERSION}+${DATE}-${PBS_ARCH}"
    VERSION_FILE="assets/python/.pbs-version"

    if [[ -f "$VERSION_FILE" ]] && [[ "$(cat "$VERSION_FILE")" == "$EXPECTED" ]]; then
        echo "Python runtime ${VERSION} (${PBS_ARCH}) already present, skipping download"
        exit 0
    fi

    FILENAME="cpython-${VERSION}+${DATE}-${PBS_ARCH}-install_only.tar.gz"
    URL="https://github.com/astral-sh/python-build-standalone/releases/download/${DATE}/${FILENAME}"

    echo "Downloading Python ${VERSION} (${PBS_ARCH}) from python-build-standalone..."
    rm -rf assets/python
    mkdir -p assets

    TMP=$(mktemp -d)
    trap "rm -rf $TMP" EXIT

    curl -fL --progress-bar "$URL" -o "$TMP/$FILENAME"
    tar xzf "$TMP/$FILENAME" -C assets/

    # Strip headers — not needed at runtime, saves ~5 MB
    rm -rf assets/python/include

    echo "$EXPECTED" > "$VERSION_FILE"
    echo "Python ${VERSION} ready at assets/python/"

dev:
    cargo run

build:
    cargo build --release

run:
    cargo run --release

# Reads .channel from CWD and installs the appropriate build.
# Run from any worktree: just install
install: fetch-python-runtime
    bash scripts/install.sh



# Wipe a channel's installed apps directory then re-sync from examples/.
# Useful when an app is renamed or removed — rsync won't delete stale dirs.
#   just clear-apps alpha
#   just clear-apps beta
#   just clear-apps stable
clear-apps channel="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "{{channel}}" ]]; then
      echo "error: channel required — one of: alpha | beta | stable"
      echo "example: just clear-apps alpha"
      exit 1
    fi
    case "{{channel}}" in
      stable) dir="$HOME/.plexi/apps"       ;;
      *)      dir="$HOME/.plexi-{{channel}}/apps" ;;
    esac
    if [[ ! -d "$dir" ]]; then
      echo "nothing to clear: $dir does not exist"
      exit 0
    fi
    count=$(find "$dir" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
    rm -rf "$dir"/*
    echo "Cleared $count app directories from $dir"
    echo "Re-run 'just install' from the matching worktree to re-sync from examples/"

bump-alpha:
    #!/usr/bin/env bash
    set -euo pipefail
    current=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    base=$(echo "$current" | sed 's/-.*//')
    IFS='.' read -r major minor patch <<< "$base"
    new="$major.$minor.$((patch + 1))"
    sed -i '' "s/^version = \"$current\"/version = \"$new\"/" Cargo.toml
    cargo generate-lockfile
    # Collect commits since last tag, stripping merge commits and chore: DEV_LOG entries
    last_tag=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
    if [[ -n "$last_tag" ]]; then
        range="$last_tag..HEAD"
    else
        range="HEAD~20..HEAD"
    fi
    mapfile -t commits < <(git log "$range" --oneline --no-merges --format="%s" | grep -v '^chore: DEV_LOG' | grep -v '^chore: bump' || true)
    today=$(date +%Y-%m-%d)
    # Build the new unreleased section
    section="## [alpha] — $today\n\n### Changes"
    for c in "${commits[@]}"; do
        section="$section\n- $c"
    done
    section="$section\n"
    # Prepend the section after the header line
    awk -v block="$section" '
        /^# Changelog/ { print; getline; print; print ""; printf "%s\n", block; next }
        { print }
    ' CHANGELOG.md > CHANGELOG.md.tmp
    mv CHANGELOG.md.tmp CHANGELOG.md
    git add Cargo.toml Cargo.lock CHANGELOG.md
    git commit -m "chore: bump alpha to $new, update changelog"
    echo "Bumped to $new"

bump:
    #!/usr/bin/env bash
    set -e
    echo "Verifying release build compiles..."
    cargo build --release
    current=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "Current version: $current"
    echo "1) prerelease (increment beta number)"
    echo "2) patch"
    echo "3) minor"
    echo "4) major"
    read -p "Bump type [1-4]: " choice
    # Strip prerelease suffix to get base version
    base=$(echo "$current" | sed 's/-.*//')
    IFS='.' read -r major minor patch <<< "$base"
    case $choice in
      1)
        # Increment prerelease counter, e.g. 3.0.0-beta.1 -> 3.0.0-beta.2
        pre=$(echo "$current" | grep -oE '\-[a-z]+\.[0-9]+$' | head -1)
        if [[ -z "$pre" ]]; then
          echo "Error: current version has no prerelease suffix to increment"
          exit 1
        fi
        pre_label=$(echo "$pre" | sed 's/-//;s/\.[0-9]*//')
        pre_num=$(echo "$pre" | grep -oE '[0-9]+$')
        new="$base-$pre_label.$((pre_num + 1))"
        ;;
      2) new="$major.$minor.$((patch + 1))" ;;
      3) new="$major.$((minor + 1)).0" ;;
      4) new="$((major + 1)).0.0" ;;
      *) echo "Invalid choice"; exit 1 ;;
    esac
    sed -i '' "s/^version = \"$current\"/version = \"$new\"/" Cargo.toml
    cargo generate-lockfile
    echo "Bumped to $new"
    git add Cargo.toml Cargo.lock
    git commit -m "chore: bump version to $new"
    git tag "v$new"
    echo "Tagged v$new"

# Promote to the next channel. Auto-detects current branch and prompts.
#   just promote        — detects alpha→beta or beta→main and confirms
#   just promote beta   — skip prompt, promote alpha→beta
#   just promote main   — skip prompt, promote beta→main
promote to="":
    bash scripts/promote.sh "{{to}}"

release:
    #!/usr/bin/env bash
    set -e
    branch=$(git rev-parse --abbrev-ref HEAD)
    version=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    tag="v$version"
    if git ls-remote --tags origin "$tag" | grep -q "$tag"; then
      echo "Error: $tag already exists on remote. Run 'just bump' first."
      exit 1
    fi
    if ! git tag -l "$tag" | grep -q "$tag"; then
      echo "Error: local tag $tag not found. Run 'just bump' first."
      exit 1
    fi
    git push origin "$branch" "$tag"
    echo "Pushed $tag from $branch — release workflow will run on GitHub Actions"
