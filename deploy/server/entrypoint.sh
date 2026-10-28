#!/bin/sh
set -eu

# CloakBrowser's stealth Chromium passes Cloudflare's Turnstile checks without
# a person, which is what comix.to's WAF asks for. Its binary license does not
# allow redistributing it in an image, so the first start downloads it into
# the data volume, checks it against the SHA-256 pinned here, and points Chek
# at it. Stock Chromium remains the fallback when the download fails.
cloak_dir="${CHEK_BROWSER_DIR:-/data/.cloakbrowser}"
cloak_bin="$cloak_dir/chrome"

# The checksums are pinned rather than read from the download site, so a
# changed file there fails the check instead of running in the server. To
# move to a new build, change the version and its checksum together; the
# next start replaces the installed copy.
case "$(uname -m)" in
    x86_64)
        cloak_version=146.0.7680.177.5
        cloak_archive=cloakbrowser-linux-x64.tar.gz
        cloak_sha256=4a12bcde95fa1bb1beef2b41ab5e5c27c36be78e3be3d0dac8c64d705216670e
        ;;
    aarch64 | arm64)
        cloak_version=146.0.7680.177.3
        cloak_archive=cloakbrowser-linux-arm64.tar.gz
        cloak_sha256=8b71ce53b4fd131327331a31fba3835d71882d19bfaabde78dd0f5390bd16f45
        ;;
    *) cloak_version= ;;
esac

install_cloak() {
    if [ -z "$cloak_version" ]; then
        echo "CloakBrowser has no build for $(uname -m)" >&2
        return 1
    fi
    tmp="$(mktemp -d "${cloak_dir}.XXXXXX")" || return 1
    result=1
    # The archive carries Chromium's data files and libraries beside the
    # binary, so extract all of it. Rename the finished directory into place,
    # so a partial install never looks complete.
    if curl -fsSL --retry 3 --retry-delay 2 \
        "https://cloakbrowser.dev/chromium-v$cloak_version/$cloak_archive" -o "$tmp/$cloak_archive" \
        && (cd "$tmp" && printf '%s  %s\n' "$cloak_sha256" "$cloak_archive" | sha256sum -c -) \
        && tar -xzf "$tmp/$cloak_archive" -C "$tmp" \
        && chmod 755 "$tmp/chrome" \
        && rm -f "$tmp/$cloak_archive" \
        && printf '%s\n' "$cloak_version" > "$tmp/VERSION" \
        && rm -rf "$cloak_dir" \
        && mv "$tmp" "$cloak_dir"; then
        result=0
    fi
    rm -rf "$tmp"
    return $result
}

installed="$(cat "$cloak_dir/VERSION" 2>/dev/null || true)"
if { [ ! -x "$cloak_bin" ] || [ "$installed" != "$cloak_version" ]; } && mkdir -p "$cloak_dir"; then
    install_cloak || echo "couldn't install CloakBrowser $cloak_version; using what's installed" >&2
fi

# Only the image's own Chromium is replaced, so a browser the admin chose
# with CHEK_BROWSER_EXECUTABLE stays in use.
if [ -x "$cloak_bin" ] && [ "${CHEK_BROWSER_EXECUTABLE:-}" = /usr/bin/chromium ]; then
    CHEK_BROWSER_EXECUTABLE="$cloak_bin"
    export CHEK_BROWSER_EXECUTABLE
fi

exec xvfb-run -a --server-args="-screen 0 1920x1080x24 -nolisten tcp" /usr/local/bin/chek "$@"
