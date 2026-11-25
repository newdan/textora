#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "textora_lite 的 .app 只能在 macOS 上打包。" >&2
    exit 1
fi

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"
target_dir="${CARGO_TARGET_DIR:-$project_root/target}"
app_name="textora_lite"
bundle_path="$target_dir/$app_name.app"
version="$(awk -F '"' '/^version = / { print $2; exit }' crates/textora-lite/Cargo.toml)"
if [[ -z "$version" ]]; then
    echo "无法读取 textora_lite 的版本号。" >&2
    exit 1
fi

cargo textora-lite --locked --target-dir "$target_dir"

staging_dir="$(mktemp -d "$target_dir/.textora_lite.XXXXXXXX")"
trap 'rm -rf -- "$staging_dir"' EXIT
staged_bundle="$staging_dir/$app_name.app"
contents="$staged_bundle/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 755 "$target_dir/lite/$app_name" "$contents/MacOS/$app_name"
# The Lite icon retains the original artwork up to 512 pixels without duplicate PNG payloads.
install -m 644 assets/TextoraLiteAppIcon.icns "$contents/Resources/AppIcon.icns"

info_plist="$contents/Info.plist"
install -m 644 assets/Info.plist "$info_plist"
for key in CFBundleExecutable CFBundleName CFBundleDisplayName; do
    plutil -replace "$key" -string "$app_name" "$info_plist"
done
plutil -replace CFBundleIdentifier -string "com.dan.textora-lite" "$info_plist"
for key in CFBundleShortVersionString CFBundleVersion; do
    plutil -replace "$key" -string "$version" "$info_plist"
done
document_type=0
while plutil -type "CFBundleDocumentTypes.$document_type" "$info_plist" >/dev/null 2>&1; do
    plutil -replace "CFBundleDocumentTypes.$document_type.LSHandlerRank" \
        -string Alternate "$info_plist"
    document_type=$((document_type + 1))
done
plutil -convert binary1 "$info_plist"
plutil -lint "$info_plist"

codesign --force --sign - "$staged_bundle"
codesign --verify --strict "$staged_bundle"

rm -rf -- "$bundle_path"
mv "$staged_bundle" "$bundle_path"
printf '已生成 %s\n' "$bundle_path"
