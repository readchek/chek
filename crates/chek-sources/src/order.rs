//! Reading order checked against chapter numbers. A source is meant to list
//! a series' chapters first to last, but sites mostly list them newest
//! first, and an extension that forgets to turn the list round makes the
//! first chapter look like the latest. Readers then find nothing after
//! chapter 1, and next and previous swap. The numbers on the chapters say
//! which way a list runs, so the server checks them rather than trusting
//! the extension.

use crate::Chapter;
use crate::ChapterOrder;

/// A chapter's number as a value to compare, when the source wrote a plain
/// one such as `12` or `12.5`.
fn number(chapter: &Chapter) -> Option<f64> {
    let number = chapter.number.as_deref()?.trim();
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return None;
    }
    number.parse().ok().filter(|value: &f64| value.is_finite())
}

/// How many steps between numbered chapters in `chapters` go up and how
/// many go down. Releases of one chapter share a number and say nothing.
fn steps(chapters: &[Chapter]) -> (usize, usize) {
    let mut rising = 0usize;
    let mut falling = 0usize;
    let mut last = None;
    for value in chapters.iter().filter_map(number) {
        if let Some(last) = last {
            if value > last {
                rising += 1;
            } else if value < last {
                falling += 1;
            }
        }
        last = Some(value);
    }
    (rising, falling)
}

/// Whether the numbered chapters in `chapters` mostly run from highest to
/// lowest. A list without two different numbers isn't taken to run either
/// way.
pub fn runs_backwards(chapters: &[Chapter]) -> bool {
    let (rising, falling) = steps(chapters);
    falling > rising
}

/// Turns a list the source handed back newest first into reading order.
pub fn into_reading_order(chapters: &mut [Chapter]) {
    if runs_backwards(chapters) {
        tracing::debug!(
            chapters = chapters.len(),
            "source listed chapters newest first; reversing"
        );
        chapters.reverse();
    }
}

/// Whether a page of chapters asked for in `order` runs the other way.
pub fn page_runs_against(chapters: &[Chapter], order: ChapterOrder) -> bool {
    let (rising, falling) = steps(chapters);
    match order {
        ChapterOrder::OldestFirst => falling > rising,
        ChapterOrder::NewestFirst => rising > falling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(id: &str, number: Option<&str>) -> Chapter {
        Chapter {
            id: id.to_owned(),
            title: id.to_owned(),
            number: number.map(str::to_owned),
            version: None,
            published: None,
            language: None,
            release_group: None,
            official: false,
        }
    }

    fn ids(chapters: &[Chapter]) -> Vec<&str> {
        chapters.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn reverses_a_newest_first_list() {
        let mut chapters = vec![
            chapter("c3", Some("3")),
            chapter("c2", Some("2")),
            chapter("c1", Some("1")),
        ];
        into_reading_order(&mut chapters);
        assert_eq!(ids(&chapters), ["c1", "c2", "c3"]);
    }

    #[test]
    fn keeps_a_list_already_in_reading_order() {
        let mut chapters = vec![
            chapter("c1", Some("1")),
            chapter("c1.5", Some("1.5")),
            chapter("extra", None),
            chapter("c2", Some("2")),
        ];
        into_reading_order(&mut chapters);
        assert_eq!(ids(&chapters), ["c1", "c1.5", "extra", "c2"]);
    }

    #[test]
    fn goes_by_most_steps_when_numbers_restart() {
        // A series whose numbering restarts at a new season still reads
        // forwards overall.
        let mut chapters = vec![
            chapter("s1c1", Some("1")),
            chapter("s1c2", Some("2")),
            chapter("s1c3", Some("3")),
            chapter("s2c1", Some("1")),
            chapter("s2c2", Some("2")),
        ];
        into_reading_order(&mut chapters);
        assert_eq!(ids(&chapters), ["s1c1", "s1c2", "s1c3", "s2c1", "s2c2"]);
    }

    #[test]
    fn ignores_releases_of_one_chapter_and_odd_numbers() {
        let mut chapters = vec![
            chapter("a2", Some("2")),
            chapter("b2", Some("02")),
            chapter("vol", Some("Vol. 9")),
            chapter("a1", Some("1")),
            chapter("b1", Some("1.0")),
        ];
        into_reading_order(&mut chapters);
        assert_eq!(ids(&chapters), ["b1", "a1", "vol", "b2", "a2"]);
    }

    #[test]
    fn leaves_a_list_without_numbers_alone() {
        let mut chapters = vec![chapter("b", None), chapter("a", Some("1"))];
        into_reading_order(&mut chapters);
        assert_eq!(ids(&chapters), ["b", "a"]);
    }

    #[test]
    fn spots_a_page_against_its_order() {
        let newest_first = [chapter("c2", Some("2")), chapter("c1", Some("1"))];
        assert!(page_runs_against(&newest_first, ChapterOrder::OldestFirst));
        assert!(!page_runs_against(&newest_first, ChapterOrder::NewestFirst));
        let single = [chapter("c1", Some("1"))];
        assert!(!page_runs_against(&single, ChapterOrder::OldestFirst));
        assert!(!page_runs_against(&single, ChapterOrder::NewestFirst));
    }
}
