#!/usr/bin/env bash
set -euo pipefail

readonly APP_NAME="notora"
readonly BINARY_NAME="notora"
readonly BUNDLE_IDENTIFIER="com.dan.notora"
readonly PACKAGE_NAME="notora-app"
readonly MINIMUM_MACOS_VERSION="12.0"

readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly TARGET_ROOT="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
readonly RELEASE_DIR="$TARGET_ROOT/release"
readonly BUNDLE_PATH="$TARGET_ROOT/${APP_NAME}.app"
readonly INFO_PLIST_SOURCE="$PROJECT_ROOT/assets/Info.plist"
readonly ICON_SOURCE="$PROJECT_ROOT/assets/AppIcon.icns"

require_macos() {
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "错误：Notora 的 .app 和 DMG 只能在 macOS 上打包。" >&2
        exit 1
    fi
}

require_file() {
    local required_path="$1"

    if [[ ! -f "$required_path" ]]; then
        echo "错误：缺少打包资源 $required_path" >&2
        exit 1
    fi
}

read_version() {
    local manifest_path="$PROJECT_ROOT/crates/notora-app/Cargo.toml"
    local package_version

    package_version="$(awk -F '"' '/^version = / { print $2; exit }' "$manifest_path")"
    if [[ -z "$package_version" ]]; then
        echo "错误：无法从 $manifest_path 读取版本号。" >&2
        exit 1
    fi

    printf '%s\n' "$package_version"
}

build_binary() {
    echo "正在构建 $PACKAGE_NAME release 二进制……"
    cargo build \
        --manifest-path "$PROJECT_ROOT/Cargo.toml" \
        --release \
        --package "$PACKAGE_NAME" \
        --bin "$BINARY_NAME"
}

create_bundle_layout() {
    local executable_source="$RELEASE_DIR/$BINARY_NAME"
    local executable_destination="$BUNDLE_PATH/Contents/MacOS/$BINARY_NAME"

    require_file "$executable_source"
    rm -rf "$BUNDLE_PATH"
    mkdir -p "$BUNDLE_PATH/Contents/MacOS" "$BUNDLE_PATH/Contents/Resources"
    install -m 755 "$executable_source" "$executable_destination"
    install -m 644 "$ICON_SOURCE" "$BUNDLE_PATH/Contents/Resources/AppIcon.icns"
}

create_info_plist() {
    local package_version="$1"
    local info_plist_destination="$BUNDLE_PATH/Contents/Info.plist"
    local plist_editor="/usr/libexec/PlistBuddy"

    require_file "$plist_editor"
    install -m 644 "$INFO_PLIST_SOURCE" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleExecutable $BINARY_NAME" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleIdentifier $BUNDLE_IDENTIFIER" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleName $APP_NAME" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleDisplayName $APP_NAME" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleShortVersionString $package_version" "$info_plist_destination"
    "$plist_editor" -c "Set :CFBundleVersion $package_version" "$info_plist_destination"
    "$plist_editor" -c "Set :LSMinimumSystemVersion $MINIMUM_MACOS_VERSION" "$info_plist_destination"
    plutil -lint "$info_plist_destination" >/dev/null
}

sign_bundle() {
    local signing_identity="${NOTORA_CODESIGN_IDENTITY:--}"

    echo "正在签名应用（身份：${signing_identity}）……"
    codesign --force --deep --options runtime --sign "$signing_identity" "$BUNDLE_PATH"
    codesign --verify --deep --strict "$BUNDLE_PATH"
}

create_disk_image() {
    local package_version="$1"
    local disk_image_path="$TARGET_ROOT/${APP_NAME}-${package_version}.dmg"

    rm -f "$disk_image_path"
    echo "正在生成 DMG……" >&2
    if ! hdiutil create \
        -fs HFS+ \
        -srcfolder "$BUNDLE_PATH" \
        -volname "$APP_NAME" \
        -format UDZO \
        "$disk_image_path" >&2; then
        rm -f "$disk_image_path"
        return 1
    fi

    printf '%s\n' "$disk_image_path"
}

create_zip_archive() {
    local package_version="$1"
    local archive_path="$TARGET_ROOT/${APP_NAME}-${package_version}-macos.zip"

    rm -f "$archive_path"
    echo "正在生成 ZIP……" >&2
    ditto -c -k --sequesterRsrc --keepParent "$BUNDLE_PATH" "$archive_path"
    unzip -tq "$archive_path" >/dev/null
    printf '%s\n' "$archive_path"
}

main() {
    local package_version
    local archive_path
    local disk_image_path=""

    require_macos
    require_file "$INFO_PLIST_SOURCE"
    require_file "$ICON_SOURCE"
    package_version="$(read_version)"

    build_binary
    create_bundle_layout
    create_info_plist "$package_version"
    sign_bundle
    archive_path="$(create_zip_archive "$package_version")"
    if ! disk_image_path="$(create_disk_image "$package_version")"; then
        echo "警告：当前环境无法生成 DMG，已保留可分发的 ZIP 包。" >&2
    fi

    echo "打包完成："
    echo "  App: $BUNDLE_PATH"
    echo "  ZIP: $archive_path"
    if [[ -n "$disk_image_path" ]]; then
        echo "  DMG: $disk_image_path"
    fi
}

main "$@"
