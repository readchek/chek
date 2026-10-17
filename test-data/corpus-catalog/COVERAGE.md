# Corpus coverage

Each directory below is a covered group with two samples, except the rooted-path
CBZ group, which has four. This groups files for selection; it does not enumerate
every possible combination of format, language, encoding, metadata, and layout.

## Covered groups

| Family       | Groups                                                                                                | Samples per group |
| ------------ | ----------------------------------------------------------------------------------------------------- | ----------------: |
| EPUB         | EPUB 2 illustrated; EPUB 2 text-focused; EPUB 3 illustrated                                           |                 2 |
| EPUB         | Fixed-layout bitmap; fixed-layout SVG; fixed-layout Japanese manga                                    |                 2 |
| EPUB         | Arabic/Hebrew RTL; vertical Japanese; embedded fonts                                                  |                 2 |
| EPUB         | Standalone/embedded previews; mixed layout; technical reference; reflowable reference; modern fiction |                 2 |
| CBZ          | Full albums, including a 421-page archive                                                             |                 2 |
| CBZ          | Existing episode archives with rooted member paths                                                    |                 4 |
| PDF          | Comic albums; modern fiction; selectable-text French novels                                           |                 2 |
| TXT          | UTF-8 English; UTF-8 Chinese; modern fiction                                                          |                 2 |
| JPEG         | Cover images; high-resolution portrait comic pages                                                    |                 2 |
| HTML/ZIP     | Standalone HTML; illustrated HTML bundles                                                             |                 2 |
| Kindle       | KF8/AZW3; older-Kindle MOBI                                                                           |                 2 |
| Documents    | Binary DOC; ODT; publisher source ZIPs                                                                |                 2 |
| Other ebooks | FB2; historic DjVu scans; Palm eReader/PDB                                                            |                 2 |

## Useful selections

- Begin with `epub/epub3-illustrated/`, `epub/epub2-text/`, and `comics/cbz-albums/`.
- Use `epub/embedded-fonts/` for long chapter lists and embedded font behavior.
  Moby Dick with Media Overlays adds synchronized audio; Kusamakura also includes
  media overlays. These are publisher-provided features, not Chek capability claims.
- Use `epub/rtl/` and `epub/vertical-japanese/` for text direction, glyph fallback,
  ruby, and non-ASCII resource names.
- Use `epub/fixed-layout-manga/`, `epub/fixed-layout-bitmap/`, and
  `epub/fixed-layout-svg/` for different fixed-layout packaging models.
- Use `epub/mixed-layout/` for per-section layout differences and the published
  scripted/text-overlay example.
- Use the original HTML ZIPs for offline HTML with assets. The separate HTML files
  intentionally retain their publisher-relative asset links.
- Little Nemo's existing CBZ has 421 image pages and macOS metadata; Pepper &
  Carrot's album has 73 image pages. Both files are unchanged.

## Not yet covered

These are gaps, not promised support or requests to implement features.

| Area                  | Missing samples or variants                                                                                                                                     |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Comic containers      | CBR/RAR4/RAR5, CB7/7z, CBT/TAR, CBA/ACE, password-protected and multipart archives                                                                              |
| Comic layouts         | Actual long-strip webtoons/manhwa, varied page dimensions, explicit ComicInfo ordering/direction variants                                                       |
| Image formats         | Standalone PNG, WebP, AVIF, GIF, TIFF, JPEG 2000, SVG, HEIF/HEIC; those codecs may occur inside other publications without constituting separate covered groups |
| Modern ebook variants | KEPUB, FB2.ZIP/FBZ, FB3, HTMLZ, TXTZ, DAISY, accessible EPUBs with differing accessibility claims                                                               |
| Office/text formats   | DOCX, RTF, Markdown, legacy encodings, UTF-16, explicit LF/CRLF variants, PML                                                                                   |
| Legacy ebooks         | PRC as a separate verified container, generic PalmDoc, Plucker, LIT, LRF/LRX, RB, TCR, SNB, CHM, XPS/OXPS                                                       |
| PDF variants          | Tagged/PDF-UA, PDF/A, forms, annotations, signatures, passwords, mixed-size and rotated pages, text-plus-OCR scan variants                                      |
| EPUB edge cases       | Obfuscated fonts, dedicated footnote/table-heavy pairs, scripted interactivity pairs, malformed files, missing resources, DRM-protected packages                |

Standard Ebooks' compatible/advanced EPUB, AZW3 and KEPUB links returned HTML rather
than ebook bytes during collection, so those responses were rejected. The author's
Sugar EPUB link returned 404; another author-published EPUB was used instead.
Download failures are recorded in the accompanying download-error JSON files.

No formats were manufactured by converting a different file or changing its
extension. No unlicensed commercial editions or ambiguous comic download sites
were used to fill gaps.
