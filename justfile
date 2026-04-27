dev:
    cargo run

build:
    cargo build --release

run:
    cargo run --release

install:
    cargo bundle --release
    cp target/release/bundle/osx/Plexi.app/Contents/MacOS/plexi /usr/local/bin/plexi
    rm -rf /Applications/Plexi.app
    cp -r target/release/bundle/osx/Plexi.app /Applications/Plexi.app

# ── Versions — full lifecycle for parallel .app installs + worktrees ─────────
#
# A "version" is a complete isolated instance:
#   - git worktree at .claude/worktrees/v-<name>/ on branch v-<name>
#   - /Applications/Plexi <name>.app   (bundled from that worktree)
#   - /usr/local/bin/plexi-<name>      (CLI)
#   - ~/.plexi-<name>/                 (config + apps + logs + secrets)
#
# Workflow:
#   just new audio-test       # create worktree + build + install + launch
#   just open audio-test      # open the installed .app
#   just ls                   # all versions with status
#   just rm audio-test        # tear down everything

# Create a new version from the current HEAD. Worktree + .app + CLI + profile.
new name:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"
    if [[ ! "$name" =~ ^[a-z0-9-]+$ ]]; then
      echo "error: name must be lowercase alphanumeric + dashes only"; exit 1
    fi
    if [[ "$name" == "alpha" || "$name" == "beta" || "$name" == "v3" || "$name" == "default" ]]; then
      echo "error: '$name' is reserved"; exit 1
    fi

    repo_root="$(git rev-parse --show-toplevel)"
    # Climb out of the current worktree to the main repo dir.
    main_git_dir="$(git rev-parse --git-common-dir)"
    main_repo="$(dirname "$main_git_dir")"
    wt_path="$main_repo/.claude/worktrees/v-$name"
    app_dest="/Applications/Plexi $name.app"

    if [[ -d "$wt_path" ]]; then echo "worktree already exists: $wt_path"; exit 1; fi
    if [[ -d "$app_dest" ]]; then echo ".app already exists: $app_dest — use 'just rm $name' first"; exit 1; fi

    echo "→ creating worktree $wt_path on branch v-$name off current HEAD"
    git worktree add -b "v-$name" "$wt_path" HEAD

    cd "$wt_path"
    # Rename the binary + bundle identity so it coexists with siblings.
    sed -i '' "s/^name = \"plexi\"/name = \"plexi-$name\"/" Cargo.toml
    sed -i '' "s/name = \"Plexi\"/name = \"Plexi $name\"/" Cargo.toml
    sed -i '' "s/identifier = \"com.ianjamesburke.plexi\"/identifier = \"com.ianjamesburke.plexi-$name\"/" Cargo.toml
    sed -i '' "s/with_title(\"Plexi\")/with_title(\"Plexi $name\")/" src/main.rs
    sed -i '' "s/\"plexi\",/\"plexi-$name\",/" src/main.rs

    echo "→ building + bundling"
    cargo bundle --release

    app_src="target/release/bundle/osx/Plexi $name.app"
    rm -rf "$app_dest"
    cp -R "$app_src" "$app_dest"
    bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    sudo cp "$bin" "/usr/local/bin/plexi-$name" 2>/dev/null || cp "$bin" "/usr/local/bin/plexi-$name"

    # Profile seeds itself on first launch via include_dir! bundled apps.
    echo ""
    echo "  ✓ worktree:  $wt_path"
    echo "  ✓ .app:      $app_dest"
    echo "  ✓ CLI:       /usr/local/bin/plexi-$name"
    echo "  ✓ profile:   ~/.plexi-$name/ (seeded on first launch)"
    echo ""
    echo "→ launching"
    open "$app_dest"

# Open an installed .app bundle. Usage: just open audio-test
open name:
    #!/usr/bin/env bash
    app="/Applications/Plexi {{name}}.app"
    if [[ ! -d "$app" ]]; then echo "not installed: $app"; exit 1; fi
    open "$app"

# List all versions with worktree + .app + profile status.
ls:
    #!/usr/bin/env bash
    echo "┌─ VERSIONS ────────────────────────────────────────────────"
    printf "  %-15s  %-8s  %-5s  %-5s  %s\n" "NAME" "BRANCH" ".APP" "CLI" "PROFILE"
    # Collect names from worktrees + .apps + profiles, dedupe.
    main_git_dir="$(git rev-parse --git-common-dir)"; main_repo="$(dirname "$main_git_dir")"
    wt_names=$(ls "$main_repo/.claude/worktrees" 2>/dev/null | sed -n 's/^v-//p' | tr '[:upper:]' '[:lower:]')
    app_names=$(ls -d "/Applications/Plexi "*.app 2>/dev/null | sed -E 's|.*/Plexi (.+)\.app|\1|' | tr '[:upper:]' '[:lower:]')
    prof_names=$(find ~ -maxdepth 1 -type d -name '.plexi-*' 2>/dev/null | sed 's|.*/\.plexi-||' | tr '[:upper:]' '[:lower:]')
    all=$(printf "%s\n%s\n%s\n" "$wt_names" "$app_names" "$prof_names" | sort -u | grep -v '^$')
    for n in $all; do
      wt="·"; app="·"; cli="·"; prof="·"; branch="·"
      if [[ -d "$main_repo/.claude/worktrees/v-$n" ]]; then
        wt="✓"; branch=$(git -C "$main_repo/.claude/worktrees/v-$n" branch --show-current 2>/dev/null)
      fi
      # .app match is case-insensitive because macOS .app folder names are capitalized.
      cap=$(echo "$n" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')
      { [[ -d "/Applications/Plexi $n.app" ]] || [[ -d "/Applications/Plexi $cap.app" ]]; } && app="✓"
      [[ -x "/usr/local/bin/plexi-$n" ]] && cli="✓"
      if [[ -d "$HOME/.plexi-$n" ]]; then
        prof=$(du -sh "$HOME/.plexi-$n" 2>/dev/null | awk '{print $1}')
      fi
      printf "  %-15s  %-8s  %-5s  %-5s  %s\n" "$n" "${branch:-·}" "$app" "$cli" "$prof"
    done
    echo "└─────────────────────────────────────────────────────────"

# Tear down a version. Removes .app, CLI, profile, worktree, branch.
rm name:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"
    if [[ "$name" == "alpha" || "$name" == "beta" || "$name" == "v3" ]]; then
      echo "error: refuse to tear down reserved '$name'"; exit 1
    fi

    main_git_dir="$(git rev-parse --git-common-dir)"; main_repo="$(dirname "$main_git_dir")"
    wt_path="$main_repo/.claude/worktrees/v-$name"
    app_dest="/Applications/Plexi $name.app"
    cli_bin="/usr/local/bin/plexi-$name"
    prof="$HOME/.plexi-$name"

    echo "Will remove:"
    [[ -d "$wt_path" ]] && echo "  - worktree   $wt_path"
    [[ -d "$app_dest" ]] && echo "  - .app       $app_dest"
    [[ -x "$cli_bin" ]] && echo "  - CLI        $cli_bin"
    [[ -d "$prof" ]] && echo "  - profile    $prof"
    read -p "Proceed? [y/N] " ok
    [[ "$ok" != "y" && "$ok" != "Y" ]] && { echo "aborted"; exit 0; }

    # Quit running instance if any.
    osascript -e "tell application \"Plexi $name\" to quit" 2>/dev/null || true
    sleep 1
    pkill -f "plexi-$name" 2>/dev/null || true

    [[ -d "$app_dest" ]] && rm -rf "$app_dest"
    [[ -x "$cli_bin" ]] && (sudo rm -f "$cli_bin" 2>/dev/null || rm -f "$cli_bin")
    [[ -d "$prof" ]] && rm -rf "$prof"
    if [[ -d "$wt_path" ]]; then
      git worktree remove --force "$wt_path"
      git branch -D "v-$name" 2>/dev/null || true
    fi
    echo "✓ torn down $name"

# Rebuild + reinstall a version from its worktree's current state.
reinstall name:
    #!/usr/bin/env bash
    set -euo pipefail
    name="{{name}}"
    main_git_dir="$(git rev-parse --git-common-dir)"; main_repo="$(dirname "$main_git_dir")"
    wt_path="$main_repo/.claude/worktrees/v-$name"
    app_dest="/Applications/Plexi $name.app"
    if [[ ! -d "$wt_path" ]]; then echo "no worktree for $name"; exit 1; fi
    cd "$wt_path"
    cargo bundle --release
    app_src="target/release/bundle/osx/Plexi $name.app"
    rm -rf "$app_dest"; cp -R "$app_src" "$app_dest"
    bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    sudo cp "$bin" "/usr/local/bin/plexi-$name" 2>/dev/null || cp "$bin" "/usr/local/bin/plexi-$name"
    echo "✓ reinstalled $name"

# ── Profiles — isolated local dev sandboxes ──────────────────────────────────
#
# Each profile lives at ~/.plexi-<name>/ with its own apps/, logs, permissions,
# and secrets index. First launch auto-creates and seeds the dir with the six
# bundled example apps. Wipe freely.

# Launch plexi-v3 with a profile (creates if missing). Usage: just p scratch
p profile:
    plexi-v3 --profile {{profile}}

# Launch from cargo run (dev build). Useful for iterating without install.
pdev profile:
    cargo run --release -- --profile {{profile}}

# List all existing profiles and their disk usage.
profiles:
    #!/usr/bin/env bash
    echo "Profiles at ~/.plexi-*:"
    find ~ -maxdepth 1 -type d -name '.plexi-*' 2>/dev/null | while read d; do
      size=$(du -sh "$d" | awk '{print $1}')
      apps=$(ls "$d/apps" 2>/dev/null | wc -l | tr -d ' ')
      name=$(basename "$d" | sed 's/^.plexi-//')
      printf "  %-20s  %s  (%s apps)\n" "$name" "$size" "$apps"
    done

# Wipe a profile completely. Usage: just pwipe scratch
pwipe profile:
    #!/usr/bin/env bash
    dir="$HOME/.plexi-{{profile}}"
    if [[ ! -d "$dir" ]]; then
      echo "Profile {{profile}} does not exist."
      exit 0
    fi
    read -p "Delete $dir? [y/N] " confirm
    if [[ "$confirm" == "y" || "$confirm" == "Y" ]]; then
      rm -rf "$dir"
      echo "Wiped $dir"
    fi

# Re-seed a profile's apps/ dir from current examples/. Destroys custom apps.
preseed profile:
    #!/usr/bin/env bash
    dir="$HOME/.plexi-{{profile}}/apps"
    rm -rf "$dir"
    mkdir -p "$dir"
    cp -R examples/. "$dir/"
    find "$dir" -name '*.py' -exec chmod +x {} \;
    echo "Re-seeded $dir ($(ls $dir | wc -l | tr -d ' ') apps)"

# Tail a profile's log file. Usage: just plogs scratch
plogs profile:
    tail -f ~/.plexi-{{profile}}/plexi.log

# Show the last 100 log lines (non-following).
ptail profile:
    tail -100 ~/.plexi-{{profile}}/plexi.log

# Open a profile's config dir in Finder.
popen profile:
    open ~/.plexi-{{profile}}/

# Deprecated: use install-alpha or install-beta or install-v3 instead
install-apps:
    #!/usr/bin/env bash
    echo "install-apps is deprecated. Use 'just install-alpha', 'just install-beta', or 'just install-v3'."
    exit 1

install-alpha:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "$(uname)" != "Darwin" ]]; then
      echo "install-alpha is macOS-only."
      exit 1
    fi

    backup_dir="$(mktemp -d)"
    cp Cargo.toml "$backup_dir/Cargo.toml"
    cp src/main.rs "$backup_dir/main.rs"

    cleanup() {
      cp "$backup_dir/Cargo.toml" Cargo.toml
      cp "$backup_dir/main.rs" src/main.rs
      rm -rf "$backup_dir"
    }
    trap cleanup EXIT

    sed -i '' 's/^name = "plexi"/name = "plexi-alpha"/' Cargo.toml
    sed -i '' 's/name = "Plexi"/name = "Plexi Alpha"/' Cargo.toml
    sed -i '' 's/identifier = "com.ianjamesburke.plexi"/identifier = "com.ianjamesburke.plexi-alpha"/' Cargo.toml
    sed -i '' 's/with_title("Plexi")/with_title("Plexi Alpha")/' src/main.rs
    sed -i '' 's/"plexi",/"plexi-alpha",/' src/main.rs

    cargo bundle --release

    app_src="target/release/bundle/osx/Plexi Alpha.app"
    app_dest="/Applications/Plexi Alpha.app"
    if [[ ! -d "$app_src" ]]; then
      echo "Error: bundle not found at $app_src"
      exit 1
    fi

    rm -rf "$app_dest"
    cp -R "$app_src" "$app_dest"

    alpha_bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    cp "$alpha_bin" /usr/local/bin/plexi-alpha

    mkdir -p ~/.plexi-alpha/sdk ~/.plexi-alpha/apps
    # Install the SDK as a proper package directory so Python can resolve
    # submodules like `plexi_sdk.ui` (SDK v2). The older flat-file layout
    # (~/.plexi-alpha/sdk/plexi_sdk.py) is removed if present.
    rm -rf ~/.plexi-alpha/sdk/plexi_sdk.py ~/.plexi-alpha/sdk/plexi_sdk
    cp -R sdk/python/plexi_sdk ~/.plexi-alpha/sdk/plexi_sdk
    find ~/.plexi-alpha/sdk/plexi_sdk -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
    rsync -a --delete examples/ ~/.plexi-alpha/apps/
    find ~/.plexi-alpha/apps -maxdepth 2 -name 'plexi_sdk.py' -delete 2>/dev/null || true
    find ~/.plexi-alpha/apps -name '*.py' -exec chmod +x {} \;

    # Refresh macOS LaunchServices so the Apple menu / app menu bar picks up
    # the current CFBundleName instead of a cached value.
    lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
    if [[ -x "$lsregister_bin" ]]; then
      "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
    fi
    /System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-alpha"
    echo "Config dir: ~/.plexi-alpha/"
    echo "Apps: $(ls ~/.plexi-alpha/apps | wc -l | tr -d ' ') synced from examples/"

install-v3:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "$(uname)" != "Darwin" ]]; then
      echo "install-v3 is macOS-only."
      exit 1
    fi

    backup_dir="$(mktemp -d)"
    cp Cargo.toml "$backup_dir/Cargo.toml"
    cp src/main.rs "$backup_dir/main.rs"

    cleanup() {
      cp "$backup_dir/Cargo.toml" Cargo.toml
      cp "$backup_dir/main.rs" src/main.rs
      rm -rf "$backup_dir"
    }
    trap cleanup EXIT

    sed -i '' 's/^name = "plexi"/name = "plexi-v3"/' Cargo.toml
    sed -i '' 's/name = "Plexi"/name = "Plexi v3"/' Cargo.toml
    sed -i '' 's/identifier = "com.ianjamesburke.plexi"/identifier = "com.ianjamesburke.plexi-v3"/' Cargo.toml
    sed -i '' 's/with_title("Plexi")/with_title("Plexi v3")/' src/main.rs
    sed -i '' 's/"plexi",/"plexi-v3",/' src/main.rs

    cargo bundle --release

    app_src="target/release/bundle/osx/Plexi v3.app"
    app_dest="/Applications/Plexi v3.app"
    if [[ ! -d "$app_src" ]]; then
      echo "Error: bundle not found at $app_src"
      exit 1
    fi

    rm -rf "$app_dest"
    cp -R "$app_src" "$app_dest"

    v3_bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    cp "$v3_bin" /usr/local/bin/plexi-v3

    mkdir -p ~/.plexi-v3/apps ~/.plexi-v3/sdk
    rm -rf ~/.plexi-v3/sdk/plexi_sdk.py ~/.plexi-v3/sdk/plexi_sdk
    cp -R sdk/python/plexi_sdk ~/.plexi-v3/sdk/plexi_sdk
    find ~/.plexi-v3/sdk/plexi_sdk -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
    rsync -a --delete examples/ ~/.plexi-v3/apps/
    # Remove any stale per-app SDK copies; apps import from ~/.plexi-v3/sdk/ via PYTHONPATH.
    find ~/.plexi-v3/apps -maxdepth 2 -name 'plexi_sdk.py' -delete
    find ~/.plexi-v3/apps -name '*.py' -exec chmod +x {} \;

    # STEP-11: refresh macOS Services registration so right-click → Services
    # shows Plexi v3 entries. CLAUDE.md used to claim these ran; now they
    # actually do.
    lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
    if [[ -x "$lsregister_bin" ]]; then
      "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
    fi
    /System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-v3"
    echo "Config dir: ~/.plexi-v3/"
    echo "Apps: $(ls ~/.plexi-v3/apps | wc -l | tr -d ' ') installed"

install-beta:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "$(uname)" != "Darwin" ]]; then
      echo "install-beta is macOS-only."
      exit 1
    fi

    backup_dir="$(mktemp -d)"
    cp Cargo.toml "$backup_dir/Cargo.toml"
    cp src/main.rs "$backup_dir/main.rs"

    cleanup() {
      cp "$backup_dir/Cargo.toml" Cargo.toml
      cp "$backup_dir/main.rs" src/main.rs
      rm -rf "$backup_dir"
    }
    trap cleanup EXIT

    sed -i '' 's/^name = "plexi"/name = "plexi-beta"/' Cargo.toml
    sed -i '' 's/name = "Plexi"/name = "Plexi Beta"/' Cargo.toml
    sed -i '' 's/identifier = "com.ianjamesburke.plexi"/identifier = "com.ianjamesburke.plexi-beta"/' Cargo.toml
    sed -i '' 's/with_title("Plexi")/with_title("Plexi Beta")/' src/main.rs
    sed -i '' 's/"plexi",/"plexi-beta",/' src/main.rs

    cargo bundle --release

    app_src="target/release/bundle/osx/Plexi Beta.app"
    app_dest="/Applications/Plexi Beta.app"
    if [[ ! -d "$app_src" ]]; then
      echo "Error: bundle not found at $app_src"
      exit 1
    fi

    rm -rf "$app_dest"
    cp -R "$app_src" "$app_dest"

    beta_bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    cp "$beta_bin" /usr/local/bin/plexi-beta

    mkdir -p ~/.plexi-beta/sdk ~/.plexi-beta/apps
    rm -rf ~/.plexi-beta/sdk/plexi_sdk.py ~/.plexi-beta/sdk/plexi_sdk
    cp -R sdk/python/plexi_sdk ~/.plexi-beta/sdk/plexi_sdk
    find ~/.plexi-beta/sdk/plexi_sdk -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
    rsync -a --delete examples/ ~/.plexi-beta/apps/
    find ~/.plexi-beta/apps -maxdepth 2 -name 'plexi_sdk.py' -delete 2>/dev/null || true
    find ~/.plexi-beta/apps -name '*.py' -exec chmod +x {} \;

    # Refresh macOS LaunchServices so the Apple menu / app menu bar picks up
    # the new CFBundleName ("Plexi Beta") instead of caching the old one.
    # Without this, the menu bar still shows "Plexi" even though the bundle
    # itself is correctly named.
    lsregister_bin="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"
    if [[ -x "$lsregister_bin" ]]; then
      "$lsregister_bin" -f "$app_dest" 2>/dev/null || echo "note: lsregister -f failed"
    fi
    /System/Library/CoreServices/pbs -update 2>/dev/null || echo "note: pbs -update failed"

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-beta"
    echo "Config dir: ~/.plexi-beta/"
    echo "Apps: $(ls ~/.plexi-beta/apps | wc -l | tr -d ' ') synced from examples/"

# Wipe a channel's installed apps directory. The install-* recipes use
# `cp -R` (sync, not mirror), so apps deleted from `examples/` persist in
# the install dir. Run this before re-installing when you want a clean
# slate — e.g. after renaming or removing an example app.
#
# Safe by design: the default `channel` arg is intentionally empty so a
# bare `just clear-apps` with no argument errors out. Pass `alpha`,
# `beta`, `v3`, or `default`.
#
#   just clear-apps alpha      # wipes ~/.plexi-alpha/apps/*
#   just clear-apps alpha && just install-alpha   # true mirror install
clear-apps channel="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "{{channel}}" ]]; then
      echo "error: channel required — one of: alpha | beta | v3 | default"
      echo "example: just clear-apps alpha"
      exit 1
    fi
    case "{{channel}}" in
      alpha)   dir="$HOME/.plexi-alpha/apps" ;;
      beta)    dir="$HOME/.plexi-beta/apps"  ;;
      v3)      dir="$HOME/.plexi-v3/apps"    ;;
      default) dir="$HOME/.plexi/apps"       ;;
      *) echo "error: unknown channel '{{channel}}' — expected alpha | beta | v3 | default"; exit 1 ;;
    esac
    if [[ ! -d "$dir" ]]; then
      echo "nothing to clear: $dir does not exist"
      exit 0
    fi
    count=$(find "$dir" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
    rm -rf "$dir"/*
    echo "Cleared $count app directories from $dir"
    echo "Re-run 'just install-{{channel}}' to re-sync from examples/"

bump:
    #!/usr/bin/env bash
    set -e
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
