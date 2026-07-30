#!/bin/bash
set -euo pipefail

# ── Configuration ──────────────────────────────────────────
APP_NAME="textora"
BUNDLE_ID="com.dan.textora"
BINARY_NAME="textora"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="$PROJECT_ROOT/target/release"
BUNDLE_DIR="$PROJECT_ROOT/target/${APP_NAME}.app"

# ── 1. Build release binary ────────────────────────────────
echo "🔨 Building release binary..."
cd "$PROJECT_ROOT"
cargo build --release

# ── 2. Create .app bundle structure ────────────────────────
echo "📦 Creating app bundle..."
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

# ── 3. Copy binary ─────────────────────────────────────────
cp "$RELEASE_DIR/$BINARY_NAME" "$BUNDLE_DIR/Contents/MacOS/"
chmod +x "$BUNDLE_DIR/Contents/MacOS/$BINARY_NAME"

# ── 4. Copy icon ───────────────────────────────────────────
if [ -f "$PROJECT_ROOT/assets/AppIcon.icns" ]; then
    cp "$PROJECT_ROOT/assets/AppIcon.icns" "$BUNDLE_DIR/Contents/Resources/"
fi

# ── 5. Generate Info.plist (inject version from Cargo.toml) ─
VERSION=$(grep -m1 '^version' "$PROJECT_ROOT/crates/app/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')
cat > "$BUNDLE_DIR/Contents/Info.plist" << PLISTEOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>${BINARY_NAME}</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleIdentifier</key>
	<string>${BUNDLE_ID}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>${APP_NAME}</string>
	<key>CFBundleDisplayName</key>
	<string>${APP_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION}</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<key>LSMinimumSystemVersion</key>
	<string>12.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
	<key>UTImportedTypeDeclarations</key>
	<array>
		<dict>
			<key>UTTypeIdentifier</key>
			<string>net.daringfireball.markdown</string>
			<key>UTTypeDescription</key>
			<string>Markdown Document</string>
			<key>UTTypeConformsTo</key>
			<array>
				<string>public.text</string>
			</array>
			<key>UTTypeTagSpecification</key>
			<dict>
				<key>public.filename-extension</key>
				<array>
					<string>md</string>
					<string>markdown</string>
					<string>mdown</string>
					<string>mkd</string>
				</array>
			</dict>
		</dict>
	</array>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Plain Text</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.plain-text</string>
				<string>public.text</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Markdown</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>net.daringfireball.markdown</string>
			</array>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>md</string>
				<string>markdown</string>
				<string>mdown</string>
				<string>mkd</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>JSON</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.json</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>XML</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.xml</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>YAML</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>yaml</string>
				<string>yml</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>TOML</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>toml</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Source Code</string>
			<key>CFBundleTypeRole</key>
			<string>Viewer</string>
			<key>LSHandlerRank</key>
			<string>Alternate</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>rs</string>
				<string>py</string>
				<string>js</string>
				<string>ts</string>
				<string>jsx</string>
				<string>tsx</string>
				<string>c</string>
				<string>h</string>
				<string>cpp</string>
				<string>hpp</string>
				<string>go</string>
				<string>java</string>
				<string>rb</string>
				<string>sh</string>
				<string>bash</string>
				<string>zsh</string>
				<string>swift</string>
				<string>css</string>
				<string>html</string>
				<string>sql</string>
				<string>lua</string>
				<string>vim</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Config File</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>ini</string>
				<string>cfg</string>
				<string>conf</string>
				<string>env</string>
				<string>properties</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>CSV</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.comma-separated-values-text</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Log File</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>log</string>
			</array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Subtitles</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>CFBundleTypeExtensions</key>
			<array>
				<string>srt</string>
				<string>vtt</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
PLISTEOF

# ── 6. Ad-hoc code sign (local dev only) ───────────────────
if command -v codesign &> /dev/null; then
    echo "🔏 Ad-hoc signing..."
    codesign --force --deep --sign - "$BUNDLE_DIR"
fi

# ── Done ───────────────────────────────────────────────────
echo ""
echo "✅ Bundle created: $BUNDLE_DIR"
echo "   Run: open \"$BUNDLE_DIR\""

# ── 7. Create DMG ─────────────────────────────────────────
# Prefer create-dmg for a styled Finder layout, but fall back
# to a plain compressed image when it fails (e.g. AppleScript
# automation is blocked on CI or in sandboxed environments).
# This avoids leaving behind large uncompressed temporary images.
DMG_PATH="$PROJECT_ROOT/target/${APP_NAME}-${VERSION}.dmg"
DMG_CREATED=false
if command -v create-dmg &> /dev/null; then
    echo "📀 Creating DMG with create-dmg..."
    rm -f "$DMG_PATH"
    if create-dmg \
        --volname "${APP_NAME}" \
        --volicon "$PROJECT_ROOT/assets/AppIcon.icns" \
        --window-size 500 350 \
        --app-drop-link 370 140 \
        --icon "${APP_NAME}.app" 130 140 \
        "$DMG_PATH" \
        "$BUNDLE_DIR"; then
        DMG_CREATED=true
    else
        echo "⚠️  create-dmg failed; falling back to hdiutil..."
    fi
else
    echo "📀 create-dmg not found; creating compressed DMG with hdiutil..."
fi

if [ "$DMG_CREATED" = false ]; then
    rm -f "$DMG_PATH"
    hdiutil create -fs HFS+ -srcfolder "$BUNDLE_DIR" \
        -volname "$APP_NAME" -format UDZO "$DMG_PATH"
fi

if [ -f "$DMG_PATH" ]; then
    echo "   DMG: $DMG_PATH"
else
    echo "⚠️  DMG creation failed"
fi

# Clean up any leftover uncompressed temporary images from create-dmg.
rm -f "$PROJECT_ROOT"/target/rw.*."${APP_NAME}-${VERSION}".dmg
