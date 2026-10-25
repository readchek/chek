#!/bin/sh
# Fills a demo server with the reader test corpus: downloads the pinned
# snapshot, lays out the Comics and Books libraries in library.list beside
# the full corpus, and creates an admin and a demo reader. Safe to run on
# every deploy; finished steps are skipped.
# Runs in the seed image's BusyBox shell, which has pipefail.
set -eu -o pipefail

CHEK_URL=${CHEK_URL:-http://chek:8080}
CORPUS_URL=${CORPUS_URL:-https://assets.chek.page}
LIBRARY=${LIBRARY:-/library}
PARTS=${PARTS:-$(dirname "$0")/corpus.parts}
MANIFEST=${MANIFEST:-$(dirname "$0")/library.list}
DOWNLOADS=$LIBRARY/downloads
FONT=${FONT:-/usr/share/fonts/dejavu/DejaVuSans.ttf}
LAID_OUT=no
ADMIN_NAME=${DEMO_ADMIN_NAME:-admin}
READER_NAME=${DEMO_READER_NAME:-demo}
: "${CHEK_SETUP_TOKEN:?set CHEK_SETUP_TOKEN}"
: "${DEMO_ADMIN_PASSWORD:?set DEMO_ADMIN_PASSWORD}"
: "${DEMO_READER_PASSWORD:?set DEMO_READER_PASSWORD}"

log() { printf 'seed: %s\n' "$*" >&2; }

matches() {
    [ -f "$1" ] && [ "$(sha256sum "$1" | cut -d ' ' -f 1)" = "$2" ]
}

fetch_corpus() {
    if [ -d "$LIBRARY/corpus" ]; then
        log "corpus already in place"
        return
    fi
    work=$LIBRARY/.download
    mkdir -p "$work"
    # Parts already downloaded and verified are kept, so an interrupted
    # download resumes where it stopped.
    grep '^part ' "$PARTS" | while read -r _ sha _ key; do
        if matches "$work/$sha" "$sha"; then
            continue
        fi
        log "downloading $key"
        curl -fsSL --retry 3 -o "$work/$sha.partial" "$CORPUS_URL/$key"
        if ! matches "$work/$sha.partial" "$sha"; then
            log "checksum mismatch for $key"
            exit 1
        fi
        mv "$work/$sha.partial" "$work/$sha"
    done
    archive_sha=$(grep '^archive ' "$PARTS" | cut -d ' ' -f 2)
    grep '^part ' "$PARTS" | cut -d ' ' -f 2 | while read -r sha; do
        cat "$work/$sha"
    done > "$work/corpus.tar.gz"
    if ! matches "$work/corpus.tar.gz" "$archive_sha"; then
        log "checksum mismatch for the joined archive"
        exit 1
    fi
    rm -rf "$work/extract"
    mkdir "$work/extract"
    tar -xzf "$work/corpus.tar.gz" -C "$work/extract" --no-same-owner --no-same-permissions
    mv "$work/extract" "$LIBRARY/corpus"
    rm -rf "$work"
    log "corpus ready"
}

# Prints the path of the verified download with the given SHA-256, fetching
# it first from the mirror and then from the upstream URL. Fails without
# exiting, so one missing file doesn't stop the rest of the demo.
download() {
    file=$DOWNLOADS/$1
    if matches "$file" "$1"; then
        printf '%s\n' "$file"
        return
    fi
    mkdir -p "$DOWNLOADS"
    # The mirror may not hold the file, so its misses are quiet.
    if curl -fsL -o "$file.partial" "$CORPUS_URL/demo-library/$1" && matches "$file.partial" "$1" ||
        curl -fsSL --retry 3 -o "$file.partial" "$2" && matches "$file.partial" "$1"; then
        mv "$file.partial" "$file"
        printf '%s\n' "$file"
        return
    fi
    rm -f "$file.partial"
    log "couldn't download $2 with SHA-256 $1"
    return 1
}

xml() {
    printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

# Binds a Book Dash picture book's pages into a CBZ, adding the credits page
# and ComicInfo.xml that its license and Book Dash's guidelines call for.
# It's called where set -e doesn't apply, so each step checks its own result.
bind_picture_book() {
    id=$1 title=$2 writer=$3 illustrator=$4 designer=$5 editor=$6 web=$7
    # Named by its manifest lines, so changing them binds the book again.
    key=$(grep -E "^(picturebook|page)\|$id\||^logo\|" "$MANIFEST" | sha256sum | cut -c 1-12)
    book=$DOWNLOADS/books/$id-$key.cbz
    if [ -f "$book" ]; then
        printf '%s\n' "$book"
        return
    fi
    work=$DOWNLOADS/books/$id.work
    rm -rf "$work"
    mkdir -p "$work" || return 1
    n=0
    while IFS='|' read -r kind page sha url; do
        [ "$kind" = page ] && [ "$page" = "$id" ] || continue
        n=$((n + 1))
        file=$(download "$sha" "$url") || return 1
        ln "$file" "$work/$(printf '%03d' "$n").jpg" || return 1
    done < "$MANIFEST"
    # ImageMagick reads \n in -annotate text as a line break.
    credits="Written by $writer\nIllustrated by $illustrator\nDesigned by $designer"
    summary="A picture book written by $writer, illustrated by $illustrator and designed by $designer"
    if [ -n "$editor" ]; then
        credits="$credits\nEdited by $editor"
        summary="A picture book written by $writer, illustrated by $illustrator, designed by $designer and edited by $editor"
    fi
    license="Originally published by Book Dash under a Creative Commons CC BY 4.0 license."
    adapted="Adapted for the Chek demo: the page images from bookdash.org, bound into a comic archive with a credits page."
    logo=$(download "$LOGO_SHA" "$LOGO_URL") || return 1
    magick -size 2048x1024 xc:white \
        \( "$logo" -resize 560x \) -gravity north -geometry +0+70 -composite \
        -font "$FONT" -fill '#1d1d1f' \
        -pointsize 60 -annotate +0+400 "$title" \
        -pointsize 38 -interline-spacing 12 -annotate +0+510 "$credits" \
        -fill '#55555a' -pointsize 30 -annotate +0+800 "$license\n${web#https://}\n$adapted" \
        -quality 90 "$work/$(printf '%03d' $((n + 1))).jpg" || return 1
    cat > "$work/ComicInfo.xml" <<XML || return 1
<?xml version="1.0" encoding="utf-8"?>
<ComicInfo>
  <Title>$(xml "$title")</Title>
  <Summary>$(xml "$summary"). $(xml "$license")</Summary>
  <Writer>$(xml "$writer")</Writer>
  <Penciller>$(xml "$illustrator")</Penciller>
  <Editor>$(xml "$editor")</Editor>
  <Publisher>Book Dash</Publisher>
  <Web>$(xml "$web")</Web>
  <LanguageISO>en</LanguageISO>
  <Notes>CC BY 4.0, https://creativecommons.org/licenses/by/4.0/. $(xml "$adapted")</Notes>
</ComicInfo>
XML
    # Stored, not deflated: the pages are already JPEG.
    rm -f "$book.partial"
    (cd "$work" && zip -q -0 -X "$book.partial" ./*.jpg ComicInfo.xml) || return 1
    mv "$book.partial" "$book" || return 1
    rm -rf "$work"
    printf '%s\n' "$book"
}

# Lays out the Comics and Books libraries from the manifest, as hard links so
# they take no extra space. Each version of the manifest gets folders of its
# own, and so libraries of its own: the server keeps files that disappear as
# unavailable rather than dropping them, so reusing a library would leave the
# old layout's books listed beside the new ones. A layout missing a file is
# laid out again on the next run.
lay_out() {
    version=$(sha256sum "$MANIFEST" | cut -c 1-12)
    LAYOUT=$LIBRARY/demo/$version
    if [ -f "$LAYOUT/.complete" ]; then
        return
    fi
    LOGO_SHA=$(grep '^logo|' "$MANIFEST" | cut -d '|' -f 2)
    LOGO_URL=$(grep '^logo|' "$MANIFEST" | cut -d '|' -f 3-)
    rm -rf "$LAYOUT.new"
    mkdir -p "$LAYOUT.new/comics" "$LAYOUT.new/books"
    complete=yes
    # The loop reads from a descriptor of its own, since the commands in it
    # read standard input.
    while IFS='|' read -r kind a b c d e f g <&3; do
        source='' to=''
        case $kind in
            corpus) source=$LIBRARY/corpus/$a to=$b ;;
            file)
                to=$b
                source=$(download "$a" "$c") || source='' ;;
            picturebook)
                # A file at the library's root is a series named after it.
                to=comics/$b.cbz
                source=$(bind_picture_book "$a" "$b" "$c" "$d" "$e" "$f" "$g") || source='' ;;
            *) continue ;;
        esac
        if [ -z "$source" ] || [ ! -f "$source" ]; then
            log "leaving out $to"
            complete=no
            continue
        fi
        mkdir -p "$(dirname "$LAYOUT.new/$to")"
        ln "$source" "$LAYOUT.new/$to"
    done 3< "$MANIFEST"
    if [ "$complete" = yes ]; then
        touch "$LAYOUT.new/.complete"
    fi
    rm -rf "$LAYOUT"
    mv "$LAYOUT.new" "$LAYOUT"
    # Earlier layouts, including the one before layouts were versioned.
    for old in "$LIBRARY"/demo/* "$LIBRARY/comics" "$LIBRARY/books"; do
        if [ -e "$old" ] && [ "$old" != "$LAYOUT" ]; then
            rm -rf "$old"
        fi
    done
    LAID_OUT=yes
    log "comics and books laid out in $LAYOUT"
}

wait_for_server() {
    tries=0
    until curl -fsS -o /dev/null "$CHEK_URL/health"; do
        tries=$((tries + 1))
        if [ "$tries" -ge 60 ]; then
            log "server at $CHEK_URL didn't answer"
            exit 1
        fi
        sleep 2
    done
}

# Prints the response body; fails on any status outside 2xx.
api() {
    curl -sS --fail-with-body -X "$1" \
        -H "authorization: Bearer $TOKEN" \
        -H 'content-type: application/json' \
        ${3:+--data "$3"} "$CHEK_URL$2"
}

sign_in() {
    credentials=$(jq -n --arg name "$ADMIN_NAME" --arg password "$DEMO_ADMIN_PASSWORD" \
        '{name: $name, password: $password, device: "demo seed"}')
    if TOKEN=$(curl -fs -H 'content-type: application/json' --data "$credentials" \
        "$CHEK_URL/api/v1/auth/sign-in" | jq -r .token); then
        return
    fi
    log "creating the admin"
    setup=$(jq -n --arg token "$CHEK_SETUP_TOKEN" --arg name "$ADMIN_NAME" \
        --arg password "$DEMO_ADMIN_PASSWORD" \
        '{setupToken: $token, name: $name, password: $password}')
    TOKEN=$(curl -sS --fail-with-body -H 'content-type: application/json' --data "$setup" \
        "$CHEK_URL/api/v1/setup" | jq -r .token)
}

# Prints the id of the library at the path, adding it when it's missing.
library() {
    id=$(api GET /api/v1/libraries | jq -r --arg path "$2" '.items[] | select(.path == $path) | .id')
    if [ -z "$id" ]; then
        log "adding library $1"
        id=$(api POST /api/v1/libraries "$(jq -n --arg name "$1" --arg path "$2" \
            '{name: $name, path: $path}')" | jq -r .id)
    fi
    printf '%s\n' "$id"
}

# Rescans a library after the layout changed. The server scans every library
# when it starts, which may still be reading the old layout, so a scan that's
# already running is waited out rather than skipped.
scan() {
    tries=0
    while :; do
        status=$(curl -sS -o /dev/null -w '%{http_code}' -X POST \
            -H "authorization: Bearer $TOKEN" "$CHEK_URL/api/v1/libraries/$1/scan")
        case $status in
            2??) return ;;
            409) ;;
            *) log "scan of library $1 answered $status"; exit 1 ;;
        esac
        tries=$((tries + 1))
        if [ "$tries" -ge 150 ]; then
            log "library $1 is still scanning"
            return
        fi
        sleep 2
    done
}

fetch_corpus
lay_out
wait_for_server
sign_in
comics=$(library Comics "$LAYOUT/comics")
books=$(library Books "$LAYOUT/books")
library "Test corpus" "$LIBRARY/corpus" > /dev/null
if [ "$LAID_OUT" = yes ]; then
    scan "$comics"
    scan "$books"
fi

# The demo reader sees the curated libraries only: the Test corpus holds
# files made to fail, which a visitor would take for bugs. An existing reader
# keeps its password, since changing it would sign testers out on every
# deploy; CHEK_DEMO_USERS stops anyone else from changing it.
grants=$(jq -n --argjson a "$comics" --argjson b "$books" '[$a, $b]')
reader=$(api GET /api/v1/users | jq -r --arg name "$READER_NAME" '.items[] | select(.name == $name) | .id')
if [ -z "$reader" ]; then
    log "creating reader $READER_NAME"
    api POST /api/v1/users "$(jq -n --arg name "$READER_NAME" \
        --arg password "$DEMO_READER_PASSWORD" --argjson grants "$grants" \
        '{name: $name, password: $password, role: "reader", libraryIds: $grants}')" > /dev/null
    reader=$(api GET /api/v1/users | jq -r --arg name "$READER_NAME" '.items[] | select(.name == $name) | .id')
else
    api PATCH "/api/v1/users/$reader" "$(jq -n --argjson grants "$grants" \
        '{libraryIds: $grants}')" > /dev/null
fi

# Only the curated public catalog belongs on the demo. Removing other
# repositories also removes their installs; private sources stay private.
catalog=https://extensions.chek.page/index.json
api GET /api/v1/extension-repositories | jq -r --arg url "$catalog" '.items[] | select(.url != $url) | .id' | while read -r repository; do
    log "removing extension repository $repository"
    api DELETE "/api/v1/extension-repositories/$repository" > /dev/null
done
repository=$(api GET /api/v1/extension-repositories | jq -r --arg url "$catalog" '.items[] | select(.url == $url) | .id')
if [ -z "$repository" ]; then
    repository=$(api POST /api/v1/extension-repositories "$(jq -n --arg url "$catalog" '{url: $url}')" | jq -r .id)
else
    api POST "/api/v1/extension-repositories/$repository/refresh" > /dev/null
fi
api PUT "/api/v1/users/$reader/extensions/page.chek.peppercarrot" \
    "$(jq -n --argjson repository "$repository" '{repositoryId: $repository}')" > /dev/null
api POST /api/v1/auth/sign-out > /dev/null
log "done"
