# Glowing Cat Software Installer

A cross-platform desktop application (Linux, macOS, Windows) for downloading and
installing the **Glowing Cat Projects** published under
[github.com/richlesh](https://github.com/richlesh).

Built in Rust with the [Slint](https://slint.dev) UI toolkit.

![status: builds + tests green on macOS](https://img.shields.io/badge/build-passing-brightgreen)

## Features

- **Project grid** — every Glowing Cat project shown as a card with icon, name,
  and short description.
- **Install / Upgrade / Remove** button per project, driven by a state machine:
  - *Not installed* → **Install**
  - *Installed, newer release available* → **Upgrade**
    - Hold **Option/Alt** to switch the button to **Remove**
  - *Installed, up to date* → **Remove**
- **OS selector** (top-left): Linux (Debian), Linux (RPM), macOS, Windows.
  Defaults to the host OS; the **architecture is auto-detected** from the running
  machine (x64 or arm64) and shown next to the selector.
- **Dark / light theme** toggle (sun/moon) in the top-right.
- **Flexible asset matching** — the GitHub release assets across the projects use
  several different naming schemes (target triples, Electron-style, Java-style,
  etc.). The matcher tokenizes filenames and scores them against the selected
  package type + architecture rather than relying on one fixed convention.

## Self-installing the app (optional)

The app can install *itself* onto the current machine from a button in the
top-right of the window ("Install this app" / "Uninstall this app"). This is
entirely optional — the app runs fine without it.

- **macOS** → `/Applications/Glowing Cat Software Installer.app` (system-wide;
  prompts for your administrator password to write to `/Applications`)
- **Linux** → `~/.local/bin/glowing-cat-installer` + a `.desktop` menu entry
  (user-scoped, no root)
- **Windows** → `%LOCALAPPDATA%\Programs\GlowingCatInstaller\` + a Start Menu shortcut
  (user-scoped, no admin)

Uninstall reverses exactly these changes. See `src/selfinstall.rs`.

## How installation works (v1)

Pressing **Install** / **Upgrade**:
1. Resolves the project's latest GitHub release.
2. Picks the best asset for the selected OS + detected architecture.
3. Downloads it to your Downloads folder.
4. Opens it with the OS so you complete installation with the native installer
   (`open` on macOS, `start` on Windows, `xdg-open` on Linux).

The installed version is recorded locally so Upgrade/Remove can reason about
state across launches.

### Path to full automation

The download-then-open design has a single, documented seam for future silent
installation: `install::open_installer`. Its doc comment lists the exact
elevated commands a future version would run per package type (`dpkg -i`,
`dnf install`, `.app` copy, `msiexec /qn`, etc.). `install::remove_project` is
the matching seam for uninstallation. Everything else — asset selection,
download, state tracking — is already in place.

## Project layout

```
Cargo.toml                      # deps + cargo-deb / cargo-generate-rpm metadata
build.rs                        # compiles ui/app.slint
ui/app.slint                    # Slint UI
src/
  main.rs                       # event loop, wiring, state machine, Alt handling
  model.rs                      # project catalog
  platform.rs                   # OS/arch detection, package-type enum
  github.rs                     # release API client + asset matcher + versions
  install.rs                    # download / open / state tracking / remove
  selfinstall.rs                # optional: install/uninstall this app itself
resources/icons/                # project icons (supply your own PNGs)
packaging/
  glowing-cat-installer.desktop # Linux desktop entry
  macos-bundle.sh               # builds .app + .dmg
  installer.iss                 # Windows Setup .exe (Inno Setup)
.github/workflows/
  ci.yml                        # build + clippy + test on Linux/macOS/Windows
  build-installers.yml          # release installers for all platforms/arches
```

## Icons

Icon paths are stubbed. Drop one PNG per project into `resources/icons/` using
the filenames listed in `resources/icons/README.md` (e.g. `spaceserval.png`).
Missing icons fall back to a built-in placeholder, so the app runs regardless.

The **application's own icon** is `resources/icons/app_icon.png`. It is used for:
the running window/taskbar icon, the macOS `.app` (`app_icon.icns`), the Windows
`.exe` (embedded via `app_icon.ico` at build time + the Inno Setup wizard icon),
and the Linux `.desktop` entry. The `.icns` and `.ico` are generated from the
PNG and committed alongside it; regenerate them if you change the PNG.

## Building

### Prerequisites

- Rust (stable). On **Linux** you also need Slint's runtime libraries:
  ```sh
  sudo apt-get install -y build-essential pkg-config \
    libfontconfig-dev libfreetype-dev libxcb1-dev libxkbcommon-dev \
    libx11-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
  ```
  macOS and Windows need no extra system packages.

### Run

```sh
cargo run --release
```

### Test / lint

```sh
cargo test --all
cargo clippy --all-targets -- -D warnings
```

## Building installers (CI)

`.github/workflows/build-installers.yml` builds the installer **for every
platform and architecture** and uploads the artifacts (and attaches them to a
GitHub Release on `v*` tags):

| Platform | Format | Architectures | Tooling |
|----------|--------|---------------|---------|
| Linux (Debian) | `.deb` | x64, arm64 | `cargo-deb` on native x64/arm64 runners |
| Linux (RPM) | `.rpm` | x64, arm64 | `cargo-generate-rpm` |
| macOS | `.dmg` | x64, arm64 | `packaging/macos-bundle.sh` + `hdiutil` |
| Windows | `Setup.exe` | x64, arm64 | Inno Setup (`packaging/installer.iss`) |

### Code signing (optional, secret-gated)

The workflow signs the macOS and Windows artifacts when the relevant repository
secrets are configured, and produces **unsigned** builds (with a Gatekeeper /
SmartScreen warning) when they are not — so forks and local builds still work.

- **macOS** — a Developer ID Application certificate (`APPLE_CERTIFICATE_BASE64`
  / `APPLE_CERTIFICATE_PASSWORD`) is imported into a temporary keychain; the
  `.app` is signed with hardened runtime + secure timestamp +
  `packaging/entitlements.plist`, and the `.dmg` is signed, submitted to Apple
  for notarization (`notarytool`, via `APPLE_API_KEY` / `APPLE_API_KEY_ID` /
  `APPLE_API_ISSUER`), and stapled.
- **Windows** — the `Setup.exe` is signed with **Azure Trusted Signing**
  (`AZURE_CLIENT_ID` / `AZURE_TENANT_ID` / `AZURE_SUBSCRIPTION_ID` +
  `AZURE_SIGNING_ENDPOINT` / `AZURE_SIGNING_ACCOUNT_NAME` /
  `AZURE_SIGNING_CERTIFICATE_PROFILE_NAME`).
- **Linux** — `.deb`/`.rpm` are not code-signed (not the platform convention here).

Trigger it by pushing a tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

or run it manually from the Actions tab (workflow_dispatch).

## Status / verification

- `cargo build --release`, `cargo test --all` (16 tests), and
  `cargo clippy --all-targets -- -D warnings` all pass on macOS arm64.
- The asset matcher was verified against the **live** GitHub releases of the
  projects, selecting the correct per-OS/per-arch asset for every distinct
  naming scheme in use.
- Linux and Windows targets build in CI (see `ci.yml`); they were authored with
  conditional compilation and have not been run on their native hardware from
  this development machine.

## License

MIT — see [LICENSE](LICENSE).
