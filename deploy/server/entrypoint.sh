#!/bin/sh
set -eu

# CloakBrowser's stealth Chromium passes Cloudflare's Turnstile checks without
# a person, which is what comix.to's WAF asks for. Its binary license does not
# allow redistributing it in an image, so the first start downloads it into
# the data volume, checks the published SHA-256, and points Chek at it. Stock
# Chromium remains the fallback when the download fails.
cloak_dir="${CHEK_BROWSER_DIR:-/data/.cloakbrowser}"
cloak_bin="$cloak_dir/chrome"

install_cloak() {
    case "$(uname -m)" in
        x86_64) version=146.0.7680.177.5; archive=cloakbrowser-linux-x64.tar.gz ;;
        aarch64 | arm64) version=146.0.7680.177.3; archive=cloakbrowser-linux-arm64.tar.gz ;;
        *)
            echo "CloakBrowser has no build for $(uname -m)" >&2
            return 1
            ;;
    esac
    base="https://cloakbrowser.dev/chromium-v$version"
    tmp="$(mktemp -d "${cloak_dir}.XXXXXX")" || return 1
    result=1
    if curl -fsSL --retry 3 --retry-delay 2 "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" \
        && curl -fsSL --retry 3 --retry-delay 2 "$base/$archive" -o "$tmp/$archive"; then
        checksum="$(grep -F "  $archive" "$tmp/SHA256SUMS")"
        # The archive carries Chromium's data files and libraries beside the
        # binary, so extract all of it. Rename the finished directory into
        # place, so a partial install never looks complete.
        if [ -n "$checksum" ] \
            && (cd "$tmp" && printf '%s\n' "$checksum" | sha256sum -c -) \
            && tar -xzf "$tmp/$archive" -C "$tmp" \
            && chmod 755 "$tmp/chrome" \
            && rm -f "$tmp/$archive" "$tmp/SHA256SUMS" \
            && rm -rf "$cloak_dir" \
            && mv "$tmp" "$cloak_dir"; then
            result=0
        fi
    fi
    rm -rf "$tmp"
    return $result
}

if [ ! -x "$cloak_bin" ] && mkdir -p "$cloak_dir"; then
    install_cloak || echo "couldn't install CloakBrowser; using stock Chromium" >&2
fi

if [ -x "$cloak_bin" ]; then
    CHEK_BROWSER_EXECUTABLE="$cloak_bin"
    export CHEK_BROWSER_EXECUTABLE
fi

exec xvfb-run -a --server-args="-screen 0 1920x1080x24 -nolisten tcp" /usr/local/bin/chek "$@"
