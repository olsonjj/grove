# Rust toolchain lives in ~/.cargo/bin; don't rely on the caller's PATH.
export PATH := env_var('HOME') / ".cargo/bin:" + env_var('PATH')

app := "target/release/bundle/osx/Grove.app"

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
    cargo test

# Format check, lint, test — what every ticket must pass
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Format code
fmt:
    cargo fmt

# Build ad-hoc-signed Grove.app
bundle:
    cargo bundle -p grove-app --release
    codesign --force --deep --sign - {{app}}
    @echo "Built {{app}}"

# Zip the Apple silicon Grove.app for a beta tester
zip: bundle
    ditto -c -k --keepParent {{app}} target/Grove.zip
    @echo "Built target/Grove.zip"

# Bundle and copy to /Applications
install: bundle
    rm -rf /Applications/Grove.app
    cp -R {{app}} /Applications/
    @echo "Installed /Applications/Grove.app"
