#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCHEME="${READER_MAC_SCHEME:-ReaderEmbeddingMac}"
DERIVED_DATA="${READER_MAC_DERIVED_DATA:-/tmp/chek-macos-derived-data}"

case "$SCHEME" in
  ReaderEmbeddingMac)
    PROJECT="$ROOT_DIR/examples/reader-consumer/ExampleApp/ReaderEmbedding.xcodeproj"
    BUNDLE_ID="dev.chek.reader.embedding.mac"
    ;;
  ChekMac)
    PROJECT="$ROOT_DIR/apps/ios/Chek.xcodeproj"
    BUNDLE_ID="run.chibi.manga.mac"
    ;;
  *)
    echo "READER_MAC_SCHEME must be ReaderEmbeddingMac or ChekMac" >&2
    exit 2
    ;;
esac

case "$MODE" in
  run|--debug|--logs|--telemetry|--verify) ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac

pkill -x "$SCHEME" >/dev/null 2>&1 || true
xcodebuild -project "$PROJECT" -scheme "$SCHEME" -configuration Debug \
  -destination 'platform=macOS,arch=arm64' -derivedDataPath "$DERIVED_DATA" \
  CODE_SIGNING_ALLOWED=NO build

APP_BUNDLE="$DERIVED_DATA/Build/Products/Debug/$SCHEME.app"
case "$MODE" in
  run) /usr/bin/open -n "$APP_BUNDLE" ;;
  --debug) lldb -- "$APP_BUNDLE/Contents/MacOS/$SCHEME" ;;
  --logs)
    /usr/bin/open -n "$APP_BUNDLE"
    /usr/bin/log stream --info --style compact --predicate "process == \"$SCHEME\""
    ;;
  --telemetry)
    /usr/bin/open -n "$APP_BUNDLE"
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify)
    /usr/bin/open -n "$APP_BUNDLE"
    sleep 1
    pgrep -x "$SCHEME" >/dev/null
    ;;
esac
