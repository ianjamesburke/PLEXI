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
