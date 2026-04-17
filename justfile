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

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-alpha"
    echo "Config dir: ~/.plexi-alpha/"

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

    mkdir -p ~/.plexi-v3/apps
    cp -R examples/. ~/.plexi-v3/apps/
    find ~/.plexi-v3/apps -name '*.py' -exec chmod +x {} \;

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

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-beta"
    echo "Config dir: ~/.plexi-beta/"

bump:
    #!/usr/bin/env bash
    set -e
    current=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
    echo "Current version: $current"
    echo "1) patch"
    echo "2) minor"
    echo "3) major"
    read -p "Bump type [1-3]: " choice
    IFS='.' read -r major minor patch <<< "$current"
    case $choice in
      1) patch=$((patch + 1)) ;;
      2) minor=$((minor + 1)); patch=0 ;;
      3) major=$((major + 1)); minor=0; patch=0 ;;
      *) echo "Invalid choice"; exit 1 ;;
    esac
    new="$major.$minor.$patch"
    sed -i '' "s/^version = \"$current\"/version = \"$new\"/" Cargo.toml
    echo "Bumped to $new"
    git add Cargo.toml
    git commit -m "chore: bump version to $new"
    git tag "v$new"
    echo "Tagged v$new"

release:
    #!/usr/bin/env bash
    set -e
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
    git push origin main "$tag"
    echo "Pushed $tag — release workflow will run on GitHub Actions"
