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

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
remap_flags="--remap-path-prefix=$cargo_home/registry/src=c --remap-path-prefix=$cargo_home/git/checkouts=g --remap-path-prefix=$project_root=w"
base_rustflags="${RUSTFLAGS:+$RUSTFLAGS }$remap_flags"

if [[ "${TEXTORA_LITE_NIGHTLY_BUILD_STD:-0}" == "1" ]]; then
    if ! command -v rustup >/dev/null 2>&1 \
        || ! rustup run nightly rustc --version >/dev/null 2>&1 \
        || ! rustup component list --toolchain nightly --installed 2>/dev/null | grep -Eq '^rust-src(-|$)'; then
        echo "启用 TEXTORA_LITE_NIGHTLY_BUILD_STD=1 需要 nightly 工具链与 rust-src 组件，请先执行：rustup toolchain install nightly --component rust-src" >&2
        exit 1
    fi
    host_triple="$(rustc -vV | awk '/^host: / { print $2 }')"
    if [[ -z "$host_triple" ]]; then
        echo "无法识别当前主机 target triple。" >&2
        exit 1
    fi
    nightly_rustflags="$base_rustflags -Z location-detail=none -Z fmt-debug=none -C force-unwind-tables=no"
    RUSTFLAGS="$nightly_rustflags" cargo +nightly build \
        --profile lite \
        --package textora_lite \
        --locked \
        --target "$host_triple" \
        --target-dir "$target_dir" \
        -Z build-std=std,panic_abort \
        -Z build-std-features=panic_immediate_abort
    binary_path="$target_dir/$host_triple/lite/$app_name"
else
    RUSTFLAGS="$base_rustflags" cargo textora-lite --locked --target-dir "$target_dir"
    binary_path="$target_dir/lite/$app_name"
fi

staging_dir="$(mktemp -d "$target_dir/.textora_lite.XXXXXXXX")"
trap 'rm -rf -- "$staging_dir"' EXIT
staged_bundle="$staging_dir/$app_name.app"
contents="$staged_bundle/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 755 "$binary_path" "$contents/MacOS/$app_name"
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
