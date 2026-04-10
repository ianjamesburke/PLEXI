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

install-apps:
    #!/usr/bin/env bash
    set -euo pipefail

    if [[ "$(uname)" != "Darwin" ]]; then
      echo "install-apps is macOS-only."
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

    sed -i '' 's/^name = "plexi"/name = "plexi-apps"/' Cargo.toml
    sed -i '' 's/name = "Plexi"/name = "Plexi Apps"/' Cargo.toml
    sed -i '' 's/identifier = "com.ianjamesburke.plexi"/identifier = "com.ianjamesburke.plexi-apps"/' Cargo.toml
    sed -i '' 's/with_title("Plexi")/with_title("Plexi Apps")/' src/main.rs
    sed -i '' 's/"plexi",/"plexi-apps",/' src/main.rs

    cargo bundle --release

    app_src="target/release/bundle/osx/Plexi Apps.app"
    app_dest="/Applications/Plexi Apps.app"
    if [[ ! -d "$app_src" ]]; then
      echo "Error: bundle not found at $app_src"
      exit 1
    fi

    rm -rf "$app_dest"
    cp -R "$app_src" "$app_dest"

    apps_bin="$(find "$app_src/Contents/MacOS" -maxdepth 1 -type f | head -n 1)"
    cp "$apps_bin" /usr/local/bin/plexi-apps

    echo "Installed $app_dest"
    echo "CLI binary: /usr/local/bin/plexi-apps"

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
