#!/bin/sh
# Fills a demo server with the reader test corpus: downloads the pinned
# snapshot, lays out Comics and Books libraries beside the full corpus, and
# creates an admin and a demo reader. Safe to run on every deploy; finished
# steps are skipped.
# Runs in the seed image's BusyBox shell, which has pipefail.
set -eu -o pipefail

CHEK_URL=${CHEK_URL:-http://chek:8080}
CORPUS_URL=${CORPUS_URL:-https://assets.chek.page}
LIBRARY=${LIBRARY:-/library}
PARTS=${PARTS:-$(dirname "$0")/corpus.parts}
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

# Hard links into the corpus, so the curated libraries take no extra space.
# Folders become series, and files at a library's root stand alone.
link() {
    mkdir -p "$(dirname "$LIBRARY/.layout/$2")"
    ln "$LIBRARY/corpus/$1" "$LIBRARY/.layout/$2"
}

lay_out() {
    if [ -d "$LIBRARY/comics" ] && [ -d "$LIBRARY/books" ]; then
        return
    fi
    rm -rf "$LIBRARY/.layout" "$LIBRARY/comics" "$LIBRARY/books"
    for n in 01 02 08 10; do
        link "edge-cases/cbz-rooted-paths/peppercarrot_episode$n.cbz" \
            "comics/Pepper & Carrot/peppercarrot_episode$n.cbz"
    done
    link comics/cbz-albums/peppercarrot-book1.cbz "comics/Pepper & Carrot Books/peppercarrot-book1.cbz"
    link pdf/comics/2020-08-02_B01_inside_72pages_8.5x11_final.pdf "comics/Pepper & Carrot Books/peppercarrot-book1.pdf"
    link pdf/comics/2020-08-02_B02_inside_72pages_8.5x11_final.pdf "comics/Pepper & Carrot Books/peppercarrot-book2.pdf"
    link comics/cbz-albums/little-nemo-1905-1914.cbz "comics/Little Nemo/little-nemo-1905-1914.cbz"
    for path in \
        epub/epub3-illustrated/alice.epub \
        epub/epub3-illustrated/pride-and-prejudice.epub \
        epub/embedded-fonts/moby-dick.epub \
        epub/modern-fiction/gorilla.epub \
        epub/modern-fiction/troll.epub \
        epub/reflowable-reference/accessible_epub_3.epub \
        epub/reflowable-reference/childrens-literature.epub \
        epub/vertical-japanese/kusamakura-japanese-vertical-writing.epub \
        epub/rtl/israelsailing.epub \
        epub/rtl/regime-anticancer-arabic.epub \
        epub/technical-reference/jlreq-in-english.epub \
        epub/fixed-layout-manga/haruko-html-jpeg.epub \
        epub/fixed-layout-bitmap/page-blanche.epub \
        epub/fixed-layout-svg/sous-le-vent.epub \
        epub/mixed-layout/cole-voyage-of-life.epub \
        pdf/modern-fiction/sugar.pdf \
        pdf/text-books/capitaine-hatteras.pdf \
        pdf/text-books/le-comte-de-chanteleine.pdf
    do
        link "$path" "books/$(basename "$path")"
    done
    mv "$LIBRARY/.layout/comics" "$LIBRARY/.layout/books" "$LIBRARY/"
    rmdir "$LIBRARY/.layout"
    log "comics and books laid out"
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

fetch_corpus
lay_out
wait_for_server
sign_in
comics=$(library Comics "$LIBRARY/comics")
books=$(library Books "$LIBRARY/books")
library "Test corpus" "$LIBRARY/corpus" > /dev/null

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
else
    api PATCH "/api/v1/users/$reader" "$(jq -n --argjson grants "$grants" \
        '{libraryIds: $grants}')" > /dev/null
fi

# The demo offers no sources: the publications are all ours to show, and
# third-party sites aren't. Removing a repository removes every install from
# it.
api GET /api/v1/extension-repositories | jq -r '.items[].id' | while read -r repository; do
    log "removing extension repository $repository"
    api DELETE "/api/v1/extension-repositories/$repository" > /dev/null
done
api POST /api/v1/auth/sign-out > /dev/null
log "done"
