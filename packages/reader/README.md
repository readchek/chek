# ChekReader

A Swift package for Chek's local EPUB and comic reader, built around native Apple
rendering and small archive support. Requires iOS 27 and Swift 6.4 or later.

Add the repository root as a local Swift package and link `ChekReader`. The root
manifest also supports Git-based installation once its commit is available remotely.
See [installation and embedding](../../docs/reader-package.md) and the
[independent consumer](../../examples/reader-consumer/) for integration details.
The package has no dependency on the Chek app, Bun, or a server. Public release
licensing and a version tag remain pending; no project source license is declared.

Current v1 scope is local EPUB and CBZ. This describes implemented behavior,
not a conformance or release certification. Runtime evidence and remaining checks
are recorded in the [validation record](../../docs/reader-package-validation.md).

| Feature         | Current behavior                                                                                                                  |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Reflowable EPUB | One XHTML/HTML spine document at a time in native WebPage; original local styles, images and fonts                                |
| Navigation      | Spine order, EPUB TOC/landmarks/page lists, EPUB 2 NCX and guide                                                                  |
| Fixed XHTML/SVG | Declared viewport fits one spine item into the available area                                                                     |
| Mixed layout    | Global rendition layout plus per-spine layout overrides                                                                           |
| Fonts           | Original local resources; standard IDPF obfuscation decoded on demand                                                             |
| Locations       | Versioned DOM/text anchors, quote context, progression fallback; fixed-page identity                                              |
| Preferences     | Selective root/body text scale with anchor restoration for reflowable content                                                     |
| Offline policy  | Publication scripts, external navigation and network resources blocked; app bridge runs in an isolated world                      |
| CBZ             | Single-frame JPEG/PNG, numeric-aware ordering, ComicInfo title/direction hints                                                    |
| Comic canvas    | Native zoom, paged swipes or continuous vertical strips; stable page/position resume                                              |
| Comic resources | Serialized on-demand decoding, cancellation, 64 MiB decoded cache; continuous canvas has a separate 64 MiB displayed-image budget |

This is incremental coverage, not a claim of EPUB conformance. Synthetic spreads,
orientation locking, EPUB 3.4 roll, Adobe obfuscation, DRM, media overlays,
audio/video, inline data resources, annotations, and full EPUB CFI are not supported.
Fixed-page pinch zoom state is not persisted. MathML, tables, bidirectional text
and embedded raster codecs depend on WebKit; broad fidelity for those features
has not been verified. EPUB spine `page-progression-direction` configures
horizontal chapter gestures: right swipes advance an `rtl` spine; left swipes
advance an `ltr`, `default` or unspecified spine. Logical spine order stays intact.
RTL forward/reverse gestures and gesture opt-out have been checked on iPhone 17
with the W3C RTL sample; default-direction navigation was rechecked afterward.
Native font rendering and accessibility
require verification against representative files and devices.
Comic page zoom magnification is also not persisted. Automatic mode uses the first
content image after the cover: an aspect ratio of at least 3 selects continuous
scrolling. This is a heuristic; callers can explicitly choose either mode.
Other comic codecs and image OCR are not implemented. Known unsupported comic
image extensions reject the archive; invalid or multi-frame JPEG/PNG data fails
when that page is loaded. The cache limit excludes a consumer's retained images and transient
ImageIO allocations; each decoded comic page is independently limited to 16
million pixels and source images to 80 million pixels / 32,768 per axis.

Imported originals and persistence belong to the app. The package opens archives
without extracting them, bounds entry/resource/XML/image processing, verifies each
loaded entry's actual size and CRC, and cancels obsolete resource requests.
Stored and Deflate ZIP entries are supported; encrypted, split and ZIP64 archives
are rejected. Archive files and declared expanded totals are each limited to
512 MiB, with at most 10,000 entries and 32 MiB per expanded entry. Parsed XML is
limited to 4 MiB, 100,000 elements and 128 levels; ComicInfo is limited to 1 MiB.
Fixed-page viewports must be positive, at most 16,384 per axis and 32 million
square units. Continuous comics allow at most 256 simultaneously visible pages.
Files or pages beyond these limits fail explicitly; the limits do not guarantee
total process memory use.

The font algorithm follows [EPUB 3.3 font obfuscation](https://www.w3.org/TR/epub-33/#sec-font-obfuscation).
Page fitting follows [fixed-layout reading-system guidance](https://www.w3.org/TR/epub-rs-33/#sec-fxl).
ZIPFoundation is pinned to 0.9.20; its MIT notice ships in the resource bundle.
