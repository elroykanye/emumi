#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$project_dir/Cargo.toml" | head -n 1)"
architecture="amd64"
package_name="emumi_${version}_${architecture}"
output_dir="$project_dir/dist"
staging_dir="$(mktemp -d -t emumi-package-XXXXXX)"
package_root="$staging_dir/$package_name"

cleanup() {
  rm -rf -- "$staging_dir"
}
trap cleanup EXIT

cargo build --manifest-path "$project_dir/Cargo.toml" --release

install -Dm755 "$project_dir/target/release/emumi" "$package_root/usr/bin/emumi"
install -Dm644 "$project_dir/packaging/dev.elroy.emumi.desktop" \
  "$package_root/usr/share/applications/dev.elroy.emumi.desktop"
install -Dm644 "$project_dir/icons/icon.png" \
  "$package_root/usr/share/icons/hicolor/256x256/apps/dev.elroy.emumi.png"
install -Dm644 "$project_dir/README.md" "$package_root/usr/share/doc/emumi/README.md"
install -Dm644 "$project_dir/CHANGELOG.md" "$package_root/usr/share/doc/emumi/CHANGELOG.md"
install -Dm644 "$project_dir/LICENSE" "$package_root/usr/share/doc/emumi/copyright"

mkdir -p "$package_root/DEBIAN" "$output_dir"
installed_size="$(du -sk "$package_root/usr" | cut -f1)"
cat > "$package_root/DEBIAN/control" <<EOF
Package: emumi
Version: $version
Section: utils
Priority: optional
Architecture: $architecture
Installed-Size: $installed_size
Depends: libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0, libgtk-3-0t64
Maintainer: Elroy
Description: Friendly Android emulator manager for Linux
 EmuMi provides a simple desktop interface for creating, cloning,
 configuring, starting and stopping Android Virtual Devices.
EOF

dpkg-deb --root-owner-group --build "$package_root" "$output_dir/${package_name}.deb"
(cd "$output_dir" && sha256sum "${package_name}.deb" > "${package_name}.deb.sha256")
printf 'Created %s\n' "$output_dir/${package_name}.deb"
