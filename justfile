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

# Deprecated: use install-alpha or install-beta instead
install-apps:
    #!/usr/bin/env bash
    echo "install-apps is deprecated. Use 'just install-alpha' or 'just install-beta' instead."
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
