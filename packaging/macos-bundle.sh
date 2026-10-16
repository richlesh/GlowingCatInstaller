#!/usr/bin/env bash
# Build a .app bundle and a .dmg for the Glowing Cat Software Installer.
#
# Usage: packaging/macos-bundle.sh <target-triple> <arch-label>
#   e.g. packaging/macos-bundle.sh aarch64-apple-darwin arm64
#        packaging/macos-bundle.sh x86_64-apple-darwin  x64
#
# Produces: dist/GlowingCatInstaller-<arch-label>.dmg
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
DMG_OUT="${ROOT}/dist/GlowingCatInstaller-${ARCH_LABEL}.dmg"

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

# Build a compressed DMG containing the .app and an Applications symlink.
STAGING="$(mktemp -d)"
cp -R "${APP_DIR}" "${STAGING}/"
ln -s /Applications "${STAGING}/Applications"
hdiutil create -volname "${APP_NAME}" -srcfolder "${STAGING}" -ov -format UDZO "${DMG_OUT}"
rm -rf "${STAGING}"

echo "Created ${DMG_OUT}"
