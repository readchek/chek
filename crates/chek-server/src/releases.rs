//! Releases of one chapter, grouped. Sites often list a chapter once per
//! group that released it, such as a publisher and a few scanlation groups,
//! so a long series reads as thousands of uploads. Browsing shows one
//! chapter per number instead, with its releases to choose from and one
//! picked: the reader's preferred group's, else an official one, else the
//! newest.
//!
//! Positions are counted in releases, not chapters, because that's what a
//! source pages by. A chapter's releases sit next to each other, so a page
//! of chapters is a run of positions and the next page starts where it ends.

use std::cmp::Reverse;
use std::collections::HashMap;

use chek_sources::Chapter;

/// What releases of one chapter share: the chapter's number, written the
/// same way however the source padded it, so `012`, `12` and `12.0` meet
/// and `12.5` stays apart. A release without a plain number has no key and
/// is a chapter of its own, so extras and specials never fold into a
/// numbered chapter.
pub fn number_key(number: Option<&str>) -> Option<String> {
    let number = number?.trim();
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole = match whole.trim_start_matches('0') {
        "" => "0",
        whole => whole,
    };
    Some(match fraction.trim_end_matches('0') {
        "" => whole.to_owned(),
        fraction => format!("{whole}.{fraction}"),
    })
}

/// Whether a release is in `language`, a BCP 47 tag. Tags match when one
/// is the other or a narrower form of it, so `en` takes `en-US` but `pt-BR`
/// doesn't take `pt-PT`. A release whose language the source doesn't give
/// always matches, since leaving it out would hide chapters.
pub fn in_language(chapter: &Chapter, language: Option<&str>) -> bool {
    let (Some(wanted), Some(have)) = (language, chapter.language.as_deref()) else {
        return true;
    };
    let (wanted, have) = (wanted.to_ascii_lowercase(), have.to_ascii_lowercase());
    let narrows = |long: &str, short: &str| {
        long.strip_prefix(short)
            .is_some_and(|rest| rest.starts_with('-'))
    };
    wanted == have || narrows(&have, &wanted) || narrows(&wanted, &have)
}

/// Puts a chapter's releases in the order they're offered, the pick first:
/// the preferred group's, then official ones, then the newest, then the
/// source's own order.
pub fn order_releases(releases: &mut [Chapter], preferred: Option<&str>) {
    let preferred = preferred.map(str::trim);
    releases.sort_by_cached_key(|release| {
        let is_preferred = preferred.is_some_and(|group| {
            release
                .release_group
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case(group))
        });
        // RFC 3339 dates sort as text; a release without one sorts last.
        (
            !is_preferred,
            !release.official,
            Reverse(release.published.clone()),
        )
    });
}

/// One chapter: its releases in the source's order, and the position of
/// the first.
#[derive(Debug)]
pub struct Group {
    pub start: usize,
    pub releases: Vec<Chapter>,
}

/// A complete list of releases, in reading order, grouped into chapters in
/// the order their first release appears, with releases of one chapter
/// next to each other wherever the source listed them. Newest first
/// (`reverse`) lists the chapters the other way round. Positions are
/// counted from the first chapter in the order given.
pub fn group_list(chapters: &[Chapter], reverse: bool) -> Vec<Group> {
    let mut groups: Vec<Vec<Chapter>> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for chapter in chapters {
        match number_key(chapter.number.as_deref()) {
            Some(key) => match by_key.get(&key) {
                Some(&index) => groups[index].push(chapter.clone()),
                None => {
                    by_key.insert(key, groups.len());
                    groups.push(vec![chapter.clone()]);
                }
            },
            None => groups.push(vec![chapter.clone()]),
        }
    }
    if reverse {
        groups.reverse();
    }
    let mut start = 0;
    groups
        .into_iter()
        .map(|releases| {
            let group = Group { start, releases };
            start += group.releases.len();
            group
        })
        .collect()
}

/// A page of chapters from `groups`: up to `limit` of them with a release
/// in `language`, starting with the chapter at `offset`, and where the next
/// page starts, if there's more.
pub fn list_page(
    groups: Vec<Group>,
    offset: usize,
    limit: usize,
    language: Option<&str>,
) -> (Vec<Group>, Option<usize>) {
    let mut page = Vec::new();
    for mut group in groups
        .into_iter()
        .skip_while(|g| g.start + g.releases.len() <= offset)
    {
        group.releases.retain(|r| in_language(r, language));
        if group.releases.is_empty() {
            continue;
        }
        if page.len() == limit {
            return (page, Some(group.start));
        }
        page.push(group);
    }
    (page, None)
}

/// How many chapters of `groups` have a release in `language`.
pub fn chapter_count(groups: &[Group], language: Option<&str>) -> usize {
    groups
        .iter()
        .filter(|g| g.releases.iter().any(|r| in_language(r, language)))
        .count()
}

/// Groups releases as a source hands them out a page at a time, trusting
/// it to list a chapter's releases next to each other.
pub struct Pager<'a> {
    limit: usize,
    language: Option<&'a str>,
    groups: Vec<(Option<String>, Group)>,
    /// Where the chapter that didn't fit starts.
    next: Option<usize>,
}

impl<'a> Pager<'a> {
    pub fn new(limit: usize, language: Option<&'a str>) -> Self {
        Self {
            limit,
            language,
            groups: Vec::new(),
            next: None,
        }
    }

    /// Adds the release at `position`. Returns false once the page is full,
    /// when this release starts a chapter past the limit.
    pub fn push(&mut self, position: usize, chapter: Chapter) -> bool {
        if !in_language(&chapter, self.language) {
            return true;
        }
        let key = number_key(chapter.number.as_deref());
        if let Some((last, group)) = self.groups.last_mut()
            && key.is_some()
            && *last == key
        {
            group.releases.push(chapter);
            return true;
        }
        if self.groups.len() == self.limit {
            self.next = Some(position);
            return false;
        }
        self.groups.push((
            key,
            Group {
                start: position,
                releases: vec![chapter],
            },
        ));
        true
    }

    /// Whether some chapter is known to be complete: one has started after
    /// it.
    pub fn has_complete(&self) -> bool {
        self.next.is_some() || self.groups.len() > 1
    }

    /// The page and where the next one starts. `read_to` is the position
    /// just past the last release read; at `end` there's nothing past it.
    /// Short of the end, the last chapter may have more releases still to
    /// come, so it's left for the next page unless it's the only one.
    pub fn finish(mut self, read_to: usize, end: bool) -> (Vec<Group>, Option<usize>) {
        let next = match self.next {
            Some(next) => Some(next),
            None if end => None,
            None if self.groups.len() > 1 => self.groups.pop().map(|(_, g)| g.start),
            None => Some(read_to),
        };
        (self.groups.into_iter().map(|(_, g)| g).collect(), next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: &str, number: Option<&str>) -> Chapter {
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

    fn ids(groups: &[Group]) -> Vec<Vec<&str>> {
        groups
            .iter()
            .map(|g| g.releases.iter().map(|r| r.id.as_str()).collect())
            .collect()
    }

    #[test]
    fn numbers_meet_however_they_are_padded() {
        assert_eq!(number_key(Some("012")).as_deref(), Some("12"));
        assert_eq!(number_key(Some("12.0")).as_deref(), Some("12"));
        assert_eq!(number_key(Some(" 12.50 ")).as_deref(), Some("12.5"));
        assert_eq!(number_key(Some("0")).as_deref(), Some("0"));
        assert_eq!(number_key(Some("12a")), None);
        assert_eq!(number_key(Some("")), None);
        assert_eq!(number_key(None), None);
    }

    #[test]
    fn languages_match_their_narrower_forms() {
        let mut chapter = release("a", Some("1"));
        chapter.language = Some("en-US".to_owned());
        assert!(in_language(&chapter, Some("en")));
        assert!(!in_language(&chapter, Some("es")));
        chapter.language = Some("pt-PT".to_owned());
        assert!(!in_language(&chapter, Some("pt-BR")));
        chapter.language = None;
        assert!(in_language(&chapter, Some("es")));
    }

    #[test]
    fn the_pick_is_preferred_then_official_then_newest() {
        let mut fan = release("fan", Some("1"));
        fan.release_group = Some("TCB Scans".to_owned());
        fan.published = Some("2026-10-20".to_owned());
        let mut viz = release("viz", Some("1"));
        viz.release_group = Some("VIZ Media".to_owned());
        viz.official = true;
        viz.published = Some("2026-10-19".to_owned());
        let mut newer = release("newer", Some("1"));
        newer.published = Some("2026-10-21".to_owned());
        let mut releases = vec![fan, viz, newer, release("undated", Some("1"))];

        order_releases(&mut releases, None);
        let order: Vec<_> = releases.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(order, ["viz", "newer", "fan", "undated"]);

        order_releases(&mut releases, Some("tcb scans"));
        assert_eq!(releases[0].id, "fan");
    }

    #[test]
    fn a_list_groups_by_number_and_pages_by_release_position() {
        let list = [
            release("1a", Some("1")),
            release("1b", Some("1.0")),
            release("1.5", Some("1.5")),
            release("extra", None),
            release("2a", Some("2")),
            release("1c", Some("01")),
        ];
        let groups = group_list(&list, false);
        assert_eq!(
            ids(&groups),
            [
                vec!["1a", "1b", "1c"],
                vec!["1.5"],
                vec!["extra"],
                vec!["2a"]
            ]
        );
        assert_eq!(chapter_count(&groups, None), 4);

        let (page, next) = list_page(group_list(&list, false), 0, 2, None);
        assert_eq!(ids(&page), [vec!["1a", "1b", "1c"], vec!["1.5"]]);
        assert_eq!(next, Some(4));
        let (page, next) = list_page(group_list(&list, false), 4, 2, None);
        assert_eq!(ids(&page), [vec!["extra"], vec!["2a"]]);
        assert_eq!(next, None);

        let (page, next) = list_page(group_list(&list, true), 0, 1, None);
        assert_eq!(ids(&page), [vec!["2a"]]);
        assert_eq!(next, Some(1));
    }

    #[test]
    fn a_paged_source_keeps_a_chapter_split_across_its_pages_whole() {
        // The source's first page ends partway through chapter 2.
        let mut pager = Pager::new(10, None);
        for (position, (id, number)) in [("1a", "1"), ("1b", "1"), ("2a", "2")]
            .into_iter()
            .enumerate()
        {
            assert!(pager.push(position, release(id, Some(number))));
        }
        assert!(pager.has_complete());
        let (page, next) = pager.finish(3, false);
        assert_eq!(ids(&page), [vec!["1a", "1b"]]);
        assert_eq!(next, Some(2));

        let mut pager = Pager::new(1, None);
        assert!(pager.push(2, release("2a", Some("2"))));
        assert!(pager.push(3, release("2b", Some("2"))));
        assert!(!pager.push(4, release("3a", Some("3"))));
        let (page, next) = pager.finish(5, false);
        assert_eq!(ids(&page), [vec!["2a", "2b"]]);
        assert_eq!(next, Some(4));
    }
}
