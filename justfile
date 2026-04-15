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
    # Register the bundle and refresh the Finder "Open in Plexi" service.
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Plexi.app || true
    /System/Library/CoreServices/pbs -update || true

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

    # Register the bundle and refresh the Finder "Open in Plexi" service.
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app_dest" || true
    /System/Library/CoreServices/pbs -update || true

    # Sync bundled example apps into ~/.plexi-alpha/apps/ so they appear
    # in the launcher. Each directory under examples/ that has a manifest.toml
    # is copied wholesale (overwriting any previous copy).
    apps_dir="$HOME/.plexi-alpha/apps"
    mkdir -p "$apps_dir"
    for dir in examples/*/; do
      if [[ -f "${dir}manifest.toml" ]]; then
        name="$(basename "$dir")"
        rm -rf "$apps_dir/$name"
        # -L dereferences symlinks (e.g. the plexi_sdk.py symlink in each
        # example dir that points to sdk/python/plexi_sdk.py). Installed apps
        # get a real bundled copy alongside their entry file — symlinks are a
        # dev-tree cleanliness convenience, not a deployment mechanism.
        cp -RL "$dir" "$apps_dir/$name"
        # Build Rust apps and place the binary where the manifest expects it.
        if [[ -f "${dir}Cargo.toml" ]]; then
          echo "Building Rust app: $name"
          (cd "$dir" && cargo build --release 2>&1)
          mkdir -p "$apps_dir/$name/bin"
          cp "${dir}target/release/plexi-app" "$apps_dir/$name/bin/plexi-app"
          chmod +x "$apps_dir/$name/bin/plexi-app"
        fi
        # Ensure Python entry points are executable (macOS strips +x on cp -R).
        find "$apps_dir/$name" -maxdepth 1 -name "*.py" -exec chmod +x {} \;
        echo "Installed app: $name"
      fi
    done

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

    # Register the bundle and refresh the Finder "Open in Plexi" service.
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app_dest" || true
    /System/Library/CoreServices/pbs -update || true

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
