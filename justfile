# Rust toolchain lives in ~/.cargo/bin; don't rely on the caller's PATH.
export PATH := env_var('HOME') / ".cargo/bin:" + env_var('PATH')

app := "target/release/bundle/osx/Grove.app"
build_file := "crates/grove-app/build-number"

# List recipes
default:
    @just --list

# Run debug build (pass a settings file to keep real settings untouched). A dev run
# never auto-checks for updates at launch; the manual menu entry still checks.
run settings="":
    GROVE_DEV=1 {{ if settings == "" { "" } else { "GROVE_SETTINGS_PATH=" + settings } }} cargo run -p grove-app

# Run release build (same dev-run behavior as `run`)
run-release:
    GROVE_DEV=1 cargo run -p grove-app --release

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

# Remove the target directory
clean:
    cargo clean

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

# Build a Developer ID–signed (hardened runtime) DMG, notarizing if creds are set.
# Requires APPLE_SIGN_IDENTITY, e.g.:
#   export APPLE_SIGN_IDENTITY=$(security find-identity -v -p codesigning | sed -E 's/.*)" (.*)"$/\2/' )
# Optional notarization envs: APPLE_API_ISSUER_ID, APPLE_API_KEY_ID, APPLE_API_KEY_PATH.
release:
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(sed -nE '/^\[workspace.package\]/,/^\[/ s/^version = "(.*)"/\1/p' Cargo.toml)
    DMG="target/Grove-$version.dmg"

    # Bump build number and bundle, mirroring the `bundle` recipe.
    echo $(( $(cat {{build_file}}) + 1 )) > {{build_file}}
    cargo bundle -p grove-app --release
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $(cat {{build_file}})" {{app}}/Contents/Info.plist

    # Sign binary-first then bundle (no --deep), hardened runtime + timestamp.
    codesign --force --options runtime --timestamp --sign "$APPLE_SIGN_IDENTITY" {{app}}
    # A "timestamps differ" failure here = clock skew (System Settings → Date & Time → auto).
    codesign --verify --strict --verbose=2 {{app}}

    hdiutil create -volname Grove -srcfolder {{app}} -ov -format UDZO "$DMG"

    # Notarize + staple when credentials are exported; otherwise signed-but-unstapled.
    if [ -n "${APPLE_API_ISSUER_ID:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
        xcrun notarytool submit "$DMG" \
            --key "$APPLE_API_KEY_PATH" \
            --key-id "$APPLE_API_KEY_ID" \
            --issuer "$APPLE_API_ISSUER_ID" \
            --wait
        xcrun stapler staple "$DMG"
        xcrun stapler validate "$DMG"
        echo "Notarized and stapled: $DMG"
    else
        echo "Signed (not notarized — no APPLE_API_* envs): $DMG"
    fi

# Zip the Apple silicon Grove.app for a beta tester
zip: bundle
    ditto -c -k --keepParent {{app}} target/Grove.zip
    @echo "Built target/Grove.zip"

# Bundle and copy to /Applications
install: bundle
    rm -rf /Applications/Grove.app
    cp -R {{app}} /Applications/
    @echo "Installed /Applications/Grove.app"
