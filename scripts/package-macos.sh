#!/bin/bash
# macOS 打包脚本：构建 Release 二进制并组装 QuickGet.app Bundle
#
# 用法：
#   ./scripts/package-macos.sh [output-dir] [binary-path]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

OUTPUT_DIR="${1:-$ROOT_DIR/dist}"
INPUT_BINARY="${2:-}"

APP_NAME="QuickGet.app"
APP_BUNDLE="$OUTPUT_DIR/$APP_NAME"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

# 动态从 Cargo.toml 提取版本号，避免硬编码不同步
VERSION=$(grep -m1 '^version' "$ROOT_DIR/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')
if [ -z "$VERSION" ]; then
    VERSION="0.1.0"
fi

if [ -n "$INPUT_BINARY" ] && [ -f "$INPUT_BINARY" ]; then
    SRC_BINARY="$INPUT_BINARY"
    echo "==> 1/4 使用指定二进制: $SRC_BINARY"
else
    echo "==> 1/4 编译 Release 二进制..."
    cd "$ROOT_DIR"
    cargo build --release
    SRC_BINARY="$ROOT_DIR/target/release/quickget"
fi

echo "==> 2/4 组装 App Bundle 目录结构 (版本: $VERSION)..."
rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

# 拷贝二进制
cp "$SRC_BINARY" "$MACOS_DIR/quickget"
chmod +x "$MACOS_DIR/quickget"

# 确保 assets/icon.icns 存在并拷贝
if [ -f "$ROOT_DIR/assets/icon.icns" ]; then
    cp "$ROOT_DIR/assets/icon.icns" "$RESOURCES_DIR/icon.icns"
elif [ -f "$ROOT_DIR/assets/icon-512.png" ]; then
    echo "正在从 icon-512.png 生成 icon.icns..."
    ICONSET_DIR="$ROOT_DIR/target/icon.iconset"
    mkdir -p "$ICONSET_DIR"
    sips -z 16 16     "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_16x16.png" >/dev/null
    sips -z 32 32     "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_16x16@2x.png" >/dev/null
    sips -z 32 32     "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_32x32.png" >/dev/null
    sips -z 64 64     "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_32x32@2x.png" >/dev/null
    sips -z 128 128   "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_128x128.png" >/dev/null
    sips -z 256 256   "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_128x128@2x.png" >/dev/null
    sips -z 256 256   "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_256x256.png" >/dev/null
    sips -z 512 512   "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_256x256@2x.png" >/dev/null
    sips -z 512 512   "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_512x512.png" >/dev/null
    sips -z 1024 1024 "$ROOT_DIR/assets/icon-512.png" --out "$ICONSET_DIR/icon_512x512@2x.png" >/dev/null
    iconutil -c icns "$ICONSET_DIR" -o "$ROOT_DIR/assets/icon.icns"
    rm -rf "$ICONSET_DIR"
    cp "$ROOT_DIR/assets/icon.icns" "$RESOURCES_DIR/icon.icns"
fi

echo "==> 3/4 写入 Info.plist..."
cat <<EOF > "$CONTENTS_DIR/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>QuickGet</string>
    <key>CFBundleDisplayName</key>
    <string>QuickGet</string>
    <key>CFBundleIdentifier</key>
    <string>com.quickget.app</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleExecutable</key>
    <string>quickget</string>
    <key>CFBundleIconFile</key>
    <string>icon.icns</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
EOF

echo "==> 4/4 写入 PkgInfo 并设置权限..."
echo -n "APPL????" > "$CONTENTS_DIR/PkgInfo"

echo "打包完成: $APP_BUNDLE"
