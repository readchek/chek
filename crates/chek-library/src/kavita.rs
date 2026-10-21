//! Syncing a Kavita source library: Kavita's series, volumes and chapters
//! are recorded in the catalog, one publication per chapter, and pages and
//! files stay on Kavita, served through it.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chek_catalog::{Catalog, ContributorRecord, PageRecord, PublicationDetails, RemoteItem};
use chek_formats::{MAX_FIELD_CHARS, MAX_SUMMARY_CHARS, Role};
use chek_kavita::{
    Chapter, Format, Kavita, KavitaError, LOOSE_LEAF_VOLUME, People, SPECIAL_VOLUME, Series,
    SeriesMetadata, Volume,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::{ScanError, ScanOptions, ScanReport, save_cover};

/// The most contributors kept per chapter, as scans keep from a file.
const MAX_CONTRIBUTORS: usize = 50;

pub(crate) async fn sync(
    catalog: Arc<Catalog>,
    library_id: i64,
    kavita: Kavita,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    let series = kavita.series().await?;
    let recorded = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.remote_versions(library_id)).await??
    };

    let mut report = ScanReport::default();
    let mut seen = HashSet::new();
    let permits = Arc::new(Semaphore::new(options.workers.max(1)));
    let covers_dir = Arc::new(options.covers_dir);
    let mut tasks = JoinSet::new();
    for series in series {
        let volumes = kavita.volumes(series.id).await?;
        // Fetched once per series, and only when a chapter in it changed.
        let mut metadata: Option<Arc<SeriesMetadata>> = None;
        let series = Arc::new(series);
        for mut volume in volumes {
            let chapters = std::mem::take(&mut volume.chapters);
            let volume = Arc::new(volume);
            for chapter in chapters {
                let Some(format) = format_of(&chapter) else {
                    debug!(
                        chapter = chapter.id,
                        "skipping Kavita chapter in a format Chek doesn't serve"
                    );
                    continue;
                };
                report.files_seen += 1;
                let remote_id = chapter.id.to_string();
                seen.insert(remote_id.clone());
                let version = version_of(&series, &chapter);
                if recorded.get(&remote_id) == Some(&version) {
                    report.files_unchanged += 1;
                    continue;
                }
                let metadata = match &metadata {
                    Some(metadata) => metadata.clone(),
                    None => {
                        let fetched = match kavita.series_metadata(series.id).await {
                            Ok(metadata) => metadata,
                            Err(e) => {
                                warn!(series = series.id, error = %e, "couldn't read Kavita series metadata");
                                SeriesMetadata::default()
                            }
                        };
                        metadata.insert(Arc::new(fetched)).clone()
                    }
                };
                let permit = permits
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("semaphore is never closed");
                let catalog = catalog.clone();
                let kavita = kavita.clone();
                let covers_dir = covers_dir.clone();
                let series = series.clone();
                let volume = volume.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let id = chapter.id;
                    let outcome = record_chapter(
                        &catalog,
                        library_id,
                        &kavita,
                        &covers_dir,
                        Placed {
                            series,
                            metadata,
                            volume,
                            chapter,
                        },
                        format,
                        version,
                    )
                    .await;
                    (id, outcome)
                });
                while let Some(done) = tasks.try_join_next() {
                    tally(&mut report, done?)?;
                }
            }
        }
    }
    while let Some(done) = tasks.join_next().await {
        tally(&mut report, done?)?;
    }

    tokio::task::spawn_blocking(move || catalog.set_remote_availability(library_id, &seen))
        .await??;
    Ok(report)
}

/// A chapter with everything around it that its record needs.
struct Placed {
    series: Arc<Series>,
    metadata: Arc<SeriesMetadata>,
    volume: Arc<Volume>,
    chapter: Chapter,
}

#[derive(Debug, thiserror::Error)]
enum ChapterError {
    #[error(transparent)]
    Kavita(#[from] KavitaError),
    #[error(transparent)]
    Catalog(#[from] chek_catalog::CatalogError),
    #[error("task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

fn tally(
    report: &mut ScanReport,
    (id, outcome): (i64, Result<(), ChapterError>),
) -> Result<(), ScanError> {
    match outcome {
        Ok(()) => report.files_recorded += 1,
        // A catalog failure would fail every later chapter too.
        Err(ChapterError::Catalog(e)) => return Err(e.into()),
        Err(ChapterError::Task(e)) => return Err(e.into()),
        Err(e) => {
            warn!(chapter = id, error = %e, "skipping Kavita chapter");
            report.files_failed += 1;
        }
    }
    Ok(())
}

async fn record_chapter(
    catalog: &Arc<Catalog>,
    library_id: i64,
    kavita: &Kavita,
    covers_dir: &Path,
    placed: Placed,
    format: &'static str,
    version: String,
) -> Result<(), ChapterError> {
    let chapter_id = placed.chapter.id;
    // Only comics are read page by page; books and PDFs are served whole.
    // Kavita doesn't list page names, so pages are numbered and the media
    // type comes with each page.
    let pages = if matches!(format, "cbz" | "cbr" | "cb7" | "cbt") {
        (0..placed.chapter.pages)
            .map(|index| PageRecord {
                entry_name: index.to_string(),
                data_offset: 0,
                compressed_size: 0,
                size: 0,
                compression: "remote",
            })
            .collect()
    } else {
        Vec::new()
    };

    let hash = blake3::hash(format!("kavita\n{library_id}\n{chapter_id}\n{version}").as_bytes())
        .to_hex()[..32]
        .to_owned();
    let cover = match kavita.cover(chapter_id).await {
        Ok(image) => {
            let covers_dir = covers_dir.to_owned();
            let name = hash.clone();
            tokio::task::spawn_blocking(move || match chek_formats::cover_from_image(&image) {
                Ok(jpeg) => save_cover(&covers_dir, &name, &jpeg),
                Err(e) => {
                    warn!(error = %e, "could not make cover from Kavita's cover");
                    None
                }
            })
            .await?
        }
        Err(e) => {
            warn!(chapter = chapter_id, error = %e, "could not fetch Kavita's cover");
            None
        }
    };

    let item = item_for(placed, format, version, hash, pages, cover);
    let catalog = catalog.clone();
    tokio::task::spawn_blocking(move || catalog.record_remote(library_id, &item)).await??;
    Ok(())
}

fn item_for(
    placed: Placed,
    format: &'static str,
    version: String,
    hash: String,
    pages: Vec<PageRecord>,
    cover: Option<String>,
) -> RemoteItem {
    let Placed {
        series,
        metadata,
        volume,
        chapter,
    } = placed;
    let series_title =
        text(&series.name, MAX_FIELD_CHARS).unwrap_or_else(|| format!("Series {}", series.id));
    let series_sort =
        text(&series.sort_name, MAX_FIELD_CHARS).unwrap_or_else(|| series_title.clone());
    let (title, number_label, sort_number) = naming(&series_title, &volume, &chapter);
    let people = if chapter.people.is_empty() {
        &metadata.people
    } else {
        &chapter.people
    };
    let details = PublicationDetails {
        summary: text(&chapter.summary, MAX_SUMMARY_CHARS)
            .or_else(|| text(&metadata.summary, MAX_SUMMARY_CHARS)),
        publisher: metadata
            .publishers
            .first()
            .and_then(|p| text(&p.name, MAX_FIELD_CHARS)),
        language: text(&chapter.language, MAX_FIELD_CHARS)
            .or_else(|| text(&metadata.language, MAX_FIELD_CHARS)),
        published: date_of(&chapter.release_date)
            .or_else(|| (metadata.release_year > 1).then(|| metadata.release_year.to_string())),
        reading_direction: None,
        contributors: contributors(people),
    };
    RemoteItem {
        remote_id: chapter.id.to_string(),
        version,
        hash,
        series_remote_id: series.id.to_string(),
        series_sort_title: chek_formats::natural_key(&series_sort),
        series_title,
        sort_title: chek_formats::natural_key(&title),
        title,
        format,
        size: chapter.files.iter().map(|f| f.bytes).sum(),
        pages,
        cover,
        number_label,
        sort_number,
        details,
    }
}

/// What to call a chapter. A chapter numbered 0 inside a real volume is the
/// whole volume's file; the rest are chapters or specials.
fn naming(
    series_title: &str,
    volume: &Volume,
    chapter: &Chapter,
) -> (String, Option<String>, Option<f64>) {
    let own_title = text(&chapter.title_name, MAX_FIELD_CHARS);
    let sort_order = (chapter.sort_order != 0.0).then_some(chapter.sort_order);
    if chapter.is_special || volume.min_number >= SPECIAL_VOLUME {
        let title = own_title
            .or_else(|| text(&chapter.title, MAX_FIELD_CHARS))
            .or_else(|| text(&chapter.range, MAX_FIELD_CHARS))
            .unwrap_or_else(|| series_title.to_owned());
        return (title, None, sort_order.map(|order| SPECIAL_VOLUME + order));
    }
    let whole_volume = chapter.number.trim() == "0" && volume.min_number > LOOSE_LEAF_VOLUME;
    if whole_volume {
        let number = text(&volume.name, MAX_FIELD_CHARS).filter(|n| n != "0");
        let title = own_title.unwrap_or_else(|| match &number {
            Some(number) => format!("Volume {number}"),
            None => series_title.to_owned(),
        });
        return (title, number, Some(volume.min_number));
    }
    let number = text(&chapter.range, MAX_FIELD_CHARS);
    let title = own_title.unwrap_or_else(|| match &number {
        Some(number) => format!("Chapter {number}"),
        None => series_title.to_owned(),
    });
    (title, number, sort_order.or(Some(chapter.min_number)))
}

fn contributors(people: &People) -> Vec<ContributorRecord> {
    let roles = [
        (&people.writers, Role::Author),
        (&people.pencillers, Role::Penciler),
        (&people.inkers, Role::Inker),
        (&people.colorists, Role::Colorist),
        (&people.letterers, Role::Letterer),
        (&people.cover_artists, Role::CoverArtist),
        (&people.editors, Role::Editor),
        (&people.translators, Role::Translator),
    ];
    roles
        .into_iter()
        .flat_map(|(people, role)| {
            people.iter().filter_map(move |p| {
                Some(ContributorRecord {
                    name: text(&p.name, MAX_FIELD_CHARS)?,
                    role: role.as_str().to_owned(),
                })
            })
        })
        .take(MAX_CONTRIBUTORS)
        .collect()
}

/// Changes when the chapter, its files or its series' name change in
/// Kavita, which is when Chek reads the chapter again.
fn version_of(series: &Series, chapter: &Chapter) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        chapter.last_modified_utc,
        chapter.files.iter().map(|f| f.bytes).sum::<u64>(),
        chapter.pages,
        series.name,
        series.sort_name,
    )
}

/// The Chek format of a chapter, from what Kavita found its file to be.
fn format_of(chapter: &Chapter) -> Option<&'static str> {
    let format = match chapter.format {
        Format::Unknown => chapter.files.first().map_or(Format::Unknown, |f| f.format),
        format => format,
    };
    Some(match format {
        // A folder of images is served page by page, and downloads as the
        // ZIP Kavita makes of it.
        Format::Image => "cbz",
        Format::Archive => {
            let path = chapter.files.first().map_or("", |f| f.file_path.as_str());
            match path
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase())
            {
                Some(ext) if ext == "cbr" || ext == "rar" => "cbr",
                Some(ext) if ext == "cb7" || ext == "7z" => "cb7",
                Some(ext) if ext == "cbt" || ext == "tar" => "cbt",
                _ => "cbz",
            }
        }
        Format::Epub => "epub",
        Format::Pdf => "pdf",
        Format::Unknown => return None,
    })
}

/// The date of a Kavita date-time, which is year 1 when there is none.
fn date_of(value: &str) -> Option<String> {
    let date = value.get(..10)?;
    let b = date.as_bytes();
    let well_formed = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    (well_formed && !date.starts_with("0001")).then(|| date.to_owned())
}

/// Trims a field and cuts it to `max` characters; blank is `None`.
fn text(value: &str, max: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(max).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(number: &str, range: &str) -> Chapter {
        serde_json::from_value(serde_json::json!({
            "id": 1, "number": number, "range": range, "minNumber": number.parse::<f64>().unwrap_or(0.0),
        }))
        .expect("a chapter")
    }

    fn volume(name: &str, min_number: f64) -> Volume {
        serde_json::from_value(
            serde_json::json!({ "id": 1, "name": name, "minNumber": min_number }),
        )
        .expect("a volume")
    }

    #[test]
    fn names_volumes_chapters_and_specials_as_kavita_shows_them() {
        let whole = naming("Blue Flag", &volume("3", 3.0), &chapter("0", "0"));
        assert_eq!(
            whole,
            ("Volume 3".to_owned(), Some("3".to_owned()), Some(3.0))
        );

        let loose = naming(
            "Blue Flag",
            &volume("-100000", LOOSE_LEAF_VOLUME),
            &chapter("12", "12"),
        );
        assert_eq!(
            loose,
            ("Chapter 12".to_owned(), Some("12".to_owned()), Some(12.0))
        );

        let mut special = chapter("0", "Omake");
        special.is_special = true;
        special.title = "Omake".to_owned();
        let special = naming("Blue Flag", &volume("100000", SPECIAL_VOLUME), &special);
        assert_eq!(special, ("Omake".to_owned(), None, None));

        // A book is one series with one unnumbered volume and chapter.
        let book = naming("The Hobbit", &volume("0", 0.0), &chapter("0", "0"));
        assert_eq!(book, ("The Hobbit".to_owned(), None, Some(0.0)));
    }
}
