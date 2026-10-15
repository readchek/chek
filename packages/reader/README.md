# ChekReader

A Swift package for Chek's local EPUB and comic reader, built around native Apple
rendering and small archive support. Requires iOS 26 and Swift 6.2 or later.

EPUB coverage implemented so far:

| Feature | Current behavior |
| --- | --- |
| Reflowable XHTML | Native WebPage, original styles, images, fonts, MathML/SVG handled by WebKit |
| Navigation | Spine order, EPUB TOC/landmarks/page lists, EPUB 2 NCX and guide |
| Fixed XHTML/SVG | Declared viewport fits one spine item into the available area |
| Mixed layout | Global rendition layout plus per-spine layout overrides |
| Fonts | Original local resources; standard IDPF obfuscation decoded on demand |
| Locations | Versioned DOM/text anchors, quote context, progression fallback; fixed-page identity |
| Preferences | Selective root/body text scale with anchor restoration for reflowable content |
| Offline policy | Scripts and network resources blocked; app bridge runs in an isolated world |
| CBZ | Single-frame JPEG/PNG, numeric-aware ordering, ComicInfo title/direction hints |
| Comic canvas | Native zoom, paged swipes or continuous vertical strips; stable page/position resume |
| Comic resources | Serialized on-demand decoding, cancellation, 64 MiB decoded cache |

This is incremental coverage, not a claim of EPUB conformance. Synthetic spreads,
orientation locking, EPUB 3.4 roll, Adobe obfuscation, DRM, media overlays,
audio/video, inline data resources, annotations, and full EPUB CFI are not supported.
Fixed-page pinch zoom state is not persisted. Native codec/font rendering and
accessibility require verification against representative files and devices.
Comic page zoom magnification is also not persisted. Automatic mode uses the first
content image after the cover: an aspect ratio of at least 3 selects continuous
scrolling. This is a heuristic; callers can explicitly choose either mode.
Other comic codecs and image OCR are not
implemented. The cache limit excludes a consumer's retained images and transient
ImageIO allocations; each decoded comic page is independently limited to 16
million pixels and source images to 80 million pixels / 32,768 per axis.

Imported originals and persistence belong to the app. The package opens archives
without extracting them, bounds entry/resource/XML/image processing, verifies each
loaded entry's actual size and CRC, and cancels obsolete resource requests.

The font algorithm follows [EPUB 3.3 font obfuscation](https://www.w3.org/TR/epub-33/#sec-font-obfuscation).
Page fitting follows [fixed-layout reading-system guidance](https://www.w3.org/TR/epub-rs-33/#sec-fxl).
ZIPFoundation is pinned to 0.9.20; its MIT notice ships in the resource bundle.
