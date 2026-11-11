# Apple signing & notarization checklist

Everything needed to ship a Developer ID–signed, notarized DMG of Grove from GitHub Actions.

Background: Grove is bundled with `cargo-bundle` (see `bundle` in the justfile) and currently
signed ad-hoc (`codesign --sign -`). Ad-hoc builds get blocked by Gatekeeper for anyone who
downloads them. A paid Apple Developer Program membership lets us ship signed + notarized
builds that open normally.

---

## 1. Developer ID Application certificate (one-time, on your Mac) — ✅ done Nov 2026

- [x] Paid Apple Developer Program membership active
      (https://developer.apple.com/account)
- [x] Keychain Access → **Certificate Assistant → Request a Certificate From a
      Certificate Authority** → save the `.certSigningRequest` to disk
      (use your dev program email) → `certs/CertificateSigningRequest.certSigningRequest`
- [x] At https://developer.apple.com/account/resources/certificates/list → **+** →
      **Developer ID Application** (⚠️ NOT "Apple Distribution" — that's App Store only)
- [x] Upload the CSR, download the `.cer` → `certs/developerID_application.cer`
- [x] Double-click the `.cer` — Keychain pairs it with your private key
      ⚠️ chain didn't validate until **Apple's Developer ID G2 intermediate** was
      imported too: `curl -O https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer`
      then `security import DeveloperIDG2CA.cer -k ~/Library/Keychains/login.keychain-db`
- [x] Verify, and record the identity + Team ID:
      ```sh
      security find-identity -v -p codesigning
      ```
      Shows: `Developer ID Application: John Olson (R9WG5SX8L3)` (hash `AF567E8D3821EE926FD83005AE86D36E837797DA`)
- [x] Record **Team ID**: `R9WG5SX8L3`

## 2. Notarization credentials — ✅ done Nov 2026

Notarization is a separate API call — Apple's servers scan the binary, then the ticket is
"stapled" onto the app/DMG locally.

- [x] App Store Connect → Users and Access → Integrations → App Store Connect API →
      generate a key with **Developer** role (lesser privilege: "App Manager")
- [x] Record **Issuer ID**: saved in `certs/issuer_id.txt` (gitignored)
- [x] Record **Key ID**: `8JM3V7GDAU` (also in `certs/key_id.txt`)
- [x] Save the `.p8` key file: `certs/AuthKey_8JM3V7GDAU.p8` (downloadable once only)

*(Simple alternative: an app-specific password from appleid.apple.com → Sign-In and
Security. Fine to start with; the API key is nicer for automation.)*

## 3. Export files for GitHub Secrets — ✅ done Nov 2026

- [x] Keychain Access: export the **Developer ID Application** cert + private key as a
      `.p12` file (set a password on export) → `certs/developerID.p12`
- [x] Create a random string for the CI keychain password → recorded below
- [x] Base64-encode the files → `certs/developerID.p12.base64`, `certs/apple-api-key.base64`
      ```sh
      base64 -i developer-id.p12          # → APPLE_CERTIFICATE_BASE64
      base64 -i AuthKey_XXXXXXXXXX.p8     # → APPLE_API_KEY_BASE64
      ```

## 4. GitHub repo secrets

Repo → Settings → Secrets and variables → Actions. Add all of:

- [ ] `APPLE_CERTIFICATE_BASE64` — base64 of the `.p12`
- [x] APPLE_CERTIFICATE_PASSWORD — password set on the `.p12` export
- [x] KEYCHAIN_PASSWORD — random string (CI creates a throwaway keychain)
- [x] APPLE_API_ISSUER_ID — `certs/issuer_id.txt` (gitignored)
- [x] APPLE_API_KEY_ID — `8JM3V7GDAU`
- [x] APPLE_API_KEY_BASE64 — base64 of the `.p8` (`certs/apple-api-key.base64`)
- [x] APPLE_TEAM_ID — `R9WG5SX8L3`

## 5. CI workflow (`release.yml`) — ✅ written: `.github/workflows/release.yml`

Runs on `macos-latest` (same runner as `check.yml`):

- [x] Import cert into a temporary keychain
      (`security import` after `base64 -d`; also fetches Apple's Developer ID G2
      intermediate — the runner root store ships only the pre-G2 CA)
- [x] `cargo bundle -p grove-app --release` + `CFBundleShortVersionString` / `CFBundleVersion` via PlistBuddy
- [x] Sign **without** `--deep` (Apple deprecates it): sign nested Mach-O first (if any), then the bundle
      `--options runtime` (hardened runtime) is **required** for notarization
- [x] Build the DMG: `hdiutil create -volname Grove -srcfolder <app> -ov -format UDZO`
- [x] Notarize + staple the **DMG** (notarytool submit --wait, stapler staple + validate)
- [x] `actions/upload-artifact@v4` with the DMG + GitHub Release (draft) on `v*` tag push

## 6. Decisions (made)

- [x] **Trigger**: `workflow_dispatch` for manual test runs + `v*` tag push for releases
- [x] **Build number**: `github.run_number` in CI — no commit-back, no conflicts;
      local `build-number` file keeps its own separate sequence
- [x] **New justfile recipe**: `just release` — Developer ID sign with
      `APPLE_SIGN_IDENTITY`, DMG, and notarize+staple when `APPLE_API_ISSUER_ID`,
      `APPLE_API_KEY_ID`, `APPLE_API_KEY_PATH` are exported

## Verified locally (Nov 2026)

- [x] Full path run once on this Mac: build → Developer ID sign (hardened runtime,
      timestamp) → DMG → `notarytool submit` **Accepted** → staple → validate ✅
      (`target/Grove-0.1.0.dmg`)
- Note: a `codesign --verify` "timestamps differ" failure means local clock skew
      (~5 min slow). Fix via System Settings → Date & Time → set automatically,
      then re-sign

## Handled? Nice-to-haves

- [ ] Test the full path once: download the DMG fresh, unzip → open → no Gatekeeper
      warning (verify with `xcrun stapler validate Grove.dmg`)
- [ ] Optional: Sparkle-style update feed or GitHub Releases hookup later
