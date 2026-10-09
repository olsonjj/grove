# Rust toolchain lives in ~/.cargo/bin; don't rely on the caller's PATH.
export PATH := env_var('HOME') / ".cargo/bin:" + env_var('PATH')

app := "target/release/bundle/osx/Grove.app"
build_file := "crates/grove-app/build-number"

# List recipes
default:
    @just --list

# Run debug build (pass a settings file to keep real settings untouched)
run settings="":
    {{ if settings == "" { "" } else { "GROVE_SETTINGS_PATH=" + settings } }} cargo run -p grove-app

# Run release build
run-release:
    cargo run -p grove-app --release

# Run all tests
test:
    cargo test --all-features --locked

# Format check, lint, test, and public API docs — what every ticket must pass
check:
    cargo fmt --check
    cargo clippy --all-targets --all-features --locked -- -D warnings
    cargo test --all-features --locked
    RUSTDOCFLAGS="-D warnings" cargo doc -p grove-core --all-features --locked --no-deps

# Format code
fmt:
    cargo fmt

# Build ad-hoc-signed Grove.app, incrementing the build number
bundle:
    echo $(( $(cat {{build_file}}) + 1 )) > {{build_file}}
    cargo bundle -p grove-app --release
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $(cat {{build_file}})" {{app}}/Contents/Info.plist
    codesign --force --deep --sign - {{app}}
    @echo "Built {{app}} $(cat {{build_file}})"

# Bump semver: just bump patch|minor|major
bump part:
    #!/usr/bin/env bash
    set -euo pipefail
    cur=$(sed -nE '/^\[workspace.package\]/,/^\[/ s/^version = "(.*)"/\1/p' Cargo.toml)
    IFS=. read -r major minor patch <<< "$cur"
    case {{part}} in
        major) next="$((major + 1)).0.0" ;;
        minor) next="$major.$((minor + 1)).0" ;;
        patch) next="$major.$minor.$((patch + 1))" ;;
        *) echo "usage: just bump patch|minor|major" >&2; exit 1 ;;
    esac
    sed -i '' -E "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$next\"/" Cargo.toml
    cargo metadata --format-version 1 > /dev/null
    echo "$cur -> $next"

# Zip the Apple silicon Grove.app for a beta tester
zip: bundle
    ditto -c -k --keepParent {{app}} target/Grove.zip
    @echo "Built target/Grove.zip"

# Bundle and copy to /Applications
install: bundle
    rm -rf /Applications/Grove.app
    cp -R {{app}} /Applications/
    @echo "Installed /Applications/Grove.app"
