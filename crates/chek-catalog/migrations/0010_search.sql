-- Full-text search over each publication's title, series, contributors and
-- publisher. The row id is the publication id; scans rewrite a
-- publication's row whenever they record its file.
CREATE VIRTUAL TABLE publication_search USING fts5 (
    title,
    series,
    contributors,
    publisher,
    tokenize = 'unicode61 remove_diacritics 2'
);

INSERT INTO publication_search (rowid, title, series, contributors, publisher)
SELECT p.id, p.title, s.title,
       (SELECT group_concat(c.name, ' ') FROM contributor c WHERE c.publication_id = p.id),
       p.publisher
FROM publication p JOIN series s ON s.id = p.series_id;
