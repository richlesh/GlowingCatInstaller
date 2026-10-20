#!/usr/bin/env bash
# Build a .app bundle and a .dmg for the Glowing Cat Software Installer.
#
# Usage: packaging/macos-bundle.sh <target-triple> <arch-label>
#   e.g. packaging/macos-bundle.sh aarch64-apple-darwin arm64
#        packaging/macos-bundle.sh x86_64-apple-darwin  x64
#
# Produces: dist/GlowingCatInstaller-<version>-<arch-label>.dmg
set -euo pipefail

TARGET="${1:?target triple required}"
ARCH_LABEL="${2:?arch label required}"

APP_NAME="Glowing Cat Software Installer"
BIN_NAME="glowing-cat-installer"
BUNDLE_ID="cloud.lesh.glowingcatinstaller"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"

ROOT="$(pwd)"
BUILD_BIN="${ROOT}/target/${TARGET}/release/${BIN_NAME}"
APP_DIR="${ROOT}/dist/${APP_NAME}.app"
DMG_OUT="${ROOT}/dist/GlowingCatInstaller-${VERSION}-${ARCH_LABEL}.dmg"

rm -rf "${APP_DIR}" "${DMG_OUT}"
mkdir -p "${APP_DIR}/Contents/MacOS" "${APP_DIR}/Contents/Resources"

cp "${BUILD_BIN}" "${APP_DIR}/Contents/MacOS/${BIN_NAME}"
chmod +x "${APP_DIR}/Contents/MacOS/${BIN_NAME}"

# Bundle the project icons alongside the binary (loaded via resources/icons).
mkdir -p "${APP_DIR}/Contents/Resources/resources/icons"
cp -R "${ROOT}/resources/icons/." "${APP_DIR}/Contents/Resources/resources/icons/" 2>/dev/null || true

# Application icon (.icns) for Finder / Dock.
ICON_LINE=""
if [ -f "${ROOT}/resources/icons/app_icon.icns" ]; then
  cp "${ROOT}/resources/icons/app_icon.icns" "${APP_DIR}/Contents/Resources/app_icon.icns"
  ICON_LINE="    <key>CFBundleIconFile</key><string>app_icon</string>"
fi

cat > "${APP_DIR}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
    <key>CFBundleExecutable</key><string>${BIN_NAME}</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
${ICON_LINE}
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# ---- Code signing (optional; CI/local) --------------------------------------
# Resolve a Developer ID Application identity to sign with, in priority order:
#   1. GCI_SIGN_IDENTITY      — a full identity string, used as-is.
#   2. GCI_SIGN_IDENTITY_NAME — the name after "Developer ID Application: ".
#   3. Auto-detect the first "Developer ID Application" identity in the keychain
#      (robust to a slightly-off secret value).
# When no identity resolves (e.g. local dev with no cert), the bundle is left
# unsigned and the build still succeeds.
ENTITLEMENTS="${ROOT}/packaging/entitlements.plist"
SIGN_IDENTITY="${GCI_SIGN_IDENTITY:-}"
if [ -z "${SIGN_IDENTITY}" ] && [ -n "${GCI_SIGN_IDENTITY_NAME:-}" ]; then
  _name="${GCI_SIGN_IDENTITY_NAME#Developer ID Application: }"
  SIGN_IDENTITY="Developer ID Application: ${_name}"
fi
KEYCHAIN_ARGS=()
if [ -n "${GCI_KEYCHAIN:-}" ]; then
  KEYCHAIN_ARGS=(--keychain "${GCI_KEYCHAIN}")
fi
if [ -z "${SIGN_IDENTITY}" ] && [ -n "${GCI_KEYCHAIN:-}" ]; then
  AUTO_ID="$(security find-identity -v -p codesigning "${GCI_KEYCHAIN}" 2>/dev/null \
    | grep -o '"Developer ID Application:[^"]*"' | head -1 | tr -d '"' || true)"
  [ -n "${AUTO_ID}" ] && SIGN_IDENTITY="${AUTO_ID}"
fi

if [ -n "${GCI_KEYCHAIN:-}" ]; then
  echo "==> Available codesigning identities in ${GCI_KEYCHAIN}:"
  security find-identity -v -p codesigning "${GCI_KEYCHAIN}" || true
fi
echo "==> Resolved signing identity: '${SIGN_IDENTITY:-<none>}'"

# Sign an .app in place with hardened runtime + secure timestamp + entitlements.
# Signs the inner executable first, then the bundle, then strict-verifies. A
# no-op when no identity is resolved.
sign_app() {
  local app="$1"
  if [ -z "${SIGN_IDENTITY}" ]; then
    echo "==> No identity; leaving ${app} unsigned"
    return 0
  fi
  echo "==> Codesigning ${app} as: ${SIGN_IDENTITY}"
  local kc=(${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"})
  codesign --force --options runtime --timestamp \
    --entitlements "${ENTITLEMENTS}" \
    --sign "${SIGN_IDENTITY}" ${kc[@]+"${kc[@]}"} \
    "${app}/Contents/MacOS/${BIN_NAME}"
  codesign --force --options runtime --timestamp \
    --entitlements "${ENTITLEMENTS}" \
    --sign "${SIGN_IDENTITY}" ${kc[@]+"${kc[@]}"} \
    "${app}"
  codesign --verify --deep --strict --verbose=2 "${app}"
  echo "==> ${app} signed and verified"
}

sign_app "${APP_DIR}"

# Build a compressed DMG containing the .app and an Applications symlink.
# Use `ditto` (not `cp -R`) so the code signature's extended attributes are
# preserved, then re-sign the staged copy so the sealed signature matches the
# exact bytes placed in the image.
STAGING="$(mktemp -d)"
ditto "${APP_DIR}" "${STAGING}/${APP_NAME}.app"
sign_app "${STAGING}/${APP_NAME}.app"
ln -s /Applications "${STAGING}/Applications"
hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGING}" -ov -format UDZO "${DMG_OUT}"
rm -rf "${STAGING}"

# Safety net: verify the signature inside the finished image when signed.
if [ -n "${SIGN_IDENTITY}" ]; then
  echo "==> Verifying signature inside the finished DMG"
  VERIFY_MNT="$(mktemp -d)"
  hdiutil attach "${DMG_OUT}" -nobrowse -readonly -mountpoint "${VERIFY_MNT}" >/dev/null
  if codesign --verify --deep --strict --verbose=2 "${VERIFY_MNT}/${APP_NAME}.app"; then
    echo "==> DMG app signature verified"
    hdiutil detach "${VERIFY_MNT}" >/dev/null
  else
    echo "ERROR: app signature invalid inside the finished DMG" >&2
    hdiutil detach "${VERIFY_MNT}" >/dev/null 2>&1 || true
    exit 1
  fi
fi

echo "Created ${DMG_OUT}"
