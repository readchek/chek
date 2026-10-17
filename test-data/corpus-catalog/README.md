# Chek reader corpus

Real publications downloaded unchanged for local reader and backend evaluation.
No synthetic books, locally converted formats, or locally repacked comic archives.
The two DOC files are original members extracted byte for byte from publisher ZIPs;
those ZIPs are retained too.

The collection contains **70 samples**, about **605 MiB**, across **14 format
families** and **34 groups**. Each covered group has at least two files. Some pairs
are different published editions or packaging variants of the same work; they are
not necessarily different stories. ZIP bundles count as a container format here.

## Contents

| Format           | Files | Examples and variants                                                                                                                                                                   |
| ---------------- | ----: | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| EPUB             |    28 | EPUB 2 and 3, illustrated and text-focused, fixed bitmap/SVG layout, Japanese manga, RTL Arabic/Hebrew, vertical Japanese, ruby, embedded fonts, media overlays, mixed layout, previews |
| CBZ              |     6 | Two full comic albums; four existing episode archives with rooted member paths                                                                                                          |
| PDF              |     6 | Color comic books, modern children's fiction, long French novels                                                                                                                        |
| TXT              |     6 | UTF-8 English, Chinese, and modern fiction                                                                                                                                              |
| JPEG             |     4 | Publisher cover images and high-resolution comic pages                                                                                                                                  |
| ZIP bundle       |     4 | Two illustrated HTML bundles and two publisher source-document bundles                                                                                                                  |
| AZW3/KF8         |     2 | Project Gutenberg's KF8 downloads                                                                                                                                                       |
| MOBI             |     2 | Project Gutenberg's older-Kindle downloads                                                                                                                                              |
| DjVu             |     2 | Historic illustrated scans of Alice and Through the Looking-Glass                                                                                                                       |
| DOC              |     2 | Original French novels in binary Word format                                                                                                                                            |
| ODT              |     2 | Author-published modern fiction                                                                                                                                                         |
| FB2              |     2 | Author-published modern fiction                                                                                                                                                         |
| HTML             |     2 | Original illustrated Gutenberg HTML; use the corresponding ZIP for offline assets                                                                                                       |
| Palm eReader/PDB |     2 | Publisher-distributed legacy eReader books                                                                                                                                              |

See [coverage and gaps](COVERAGE.md) before assuming a particular variant is present.
The corpus does not establish that Chek supports these formats.

## Sources and rights

- [Project Gutenberg](https://www.gutenberg.org/): original-language public-domain classics; original Gutenberg notices remain in the files.
- [IDPF EPUB 3 Samples](https://idpf.github.io/epub3-samples/30/samples.html): published interoperability examples. The collection specifies CC BY-SA 3.0 unless otherwise stated; per-book rights and component notices are retained. Israel Sailing specifies CC BY-SA 2.5. The Japanese layout requirements books retain the W3C document license.
- [Pepper & Carrot](https://www.peppercarrot.com/en/about/index.html): David Revoy and credited contributors, CC BY 4.0. Comic PDFs and JPEGs come from the publisher; episode CBZs come from the Internet Archive. The album CBZ is an existing distribution from [Pulpit](https://github.com/vincentarelbundock/pulpit/blob/main/LICENSES/README.md).
- [Crimperbooks](https://www.crimperbooks.co.uk/legal/): Ryan P. Cartwright, CC BY-SA 4.0, downloaded from the author's linked storage.
- [Ebooks libres et gratuits](https://www.ebooksgratuits.com/ebooks.php?auteur=Verne_Jules): Jules Verne's original French works. Publisher notices remain in the files; the [source's copyright explanation](https://www.ebooksgratuits.com/droitaut.php) is saved with the corpus.
- Wikimedia Commons: historic Carroll scans, with the original editions, authors, illustrators, and public-domain rationale recorded on the source pages.
- [Little Nemo 1905–1914](https://archive.org/details/LittleNemo1905-14FromWinsorMccayAsCbz): historic strips by Winsor McCay, downloaded as the existing 421-page CBZ. The archive item is marked CC0/public domain.

This is a mixed-license collection. The repository's license does not replace
individual files' terms. Keep the original notices and the accompanying provenance
when sharing files. Attribution applies to CC BY and CC BY-SA works, and their
respective share-alike conditions apply to adaptations. The files were not edited.

## Catalog and integrity

- [manifest.json](manifest.json): titles, creators, URLs, rights, sizes, hashes, groups, and embedded EPUB rights notices.
- [SHA256SUMS](SHA256SUMS): checksums for the 70 reading samples.
- [verification.json](verification.json): detected file types, archive CRCs, EPUB structure/resources, PDF page counts, image dimensions, and archive-path observations.
- [provenance/](provenance/): downloaded source pages, archive metadata, attribution records, and license texts.

ZIP CRCs were checked; EPUB container and package documents were parsed with all
manifest resources found; PDFs and FB2/ODT structures were parsed; standalone JPEGs
and representative comic images were decoded. This is file integrity evidence,
not EPUBCheck certification or verification in a running Chek app. DjVu, DOC, and
PDB are identified by file signatures and have not been rendered in a reader.

`edge-cases/cbz-rooted-paths/` contains real downloads with leading-slash member
names. Treat those as archive-path cases, not extraction destinations. The Little
Nemo album also includes macOS resource-fork metadata that a reader should ignore.

Collected on 2026-10-17. Nothing here needs a backend connection to inspect.
