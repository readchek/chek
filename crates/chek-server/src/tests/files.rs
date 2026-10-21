//! Serving pages from library files, and uploading files into libraries.

use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH};

use super::{BLUE_PNG, RED_PNG, Server, assert_error, body, cbz};

#[tokio::test(flavor = "multi_thread")]
async fn pages_come_from_the_scanned_file_until_it_changes() {
    let server = Server::start().await;
    let admin = server.admin().await;
    // Entries are written out of order to check pages are sorted by name.
    let book = cbz(&[("02.png", &BLUE_PNG), ("01.png", &RED_PNG)]);
    let folder = server.folder("comics", &[("book.cbz", &book)]);
    let library = server.folder_library(&admin, "Comics", &folder).await;

    let publications = server.publications(&admin, library).await;
    assert_eq!(publications.len(), 1, "{publications:?}");
    assert_eq!(publications[0]["format"], "cbz");
    assert_eq!(publications[0]["pageCount"], 2);
    let id = publications[0]["id"].as_i64().unwrap();
    let page = |index: u32| server.get(&format!("/api/v1/publications/{id}/pages/{index}"));

    let response = page(0).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    let etag = response.headers()[ETAG].clone();
    assert_eq!(response.bytes().await.unwrap().as_ref(), RED_PNG);
    let response = page(1).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), BLUE_PNG);
    let response = page(2).bearer_auth(&admin).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = page(0)
        .bearer_auth(&admin)
        .header(IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    // The file is replaced by one of another size, so the stored offsets no
    // longer describe it. Clients get a conflict instead of bytes from the
    // wrong place, and the rescan the conflict starts fixes the index.
    let replaced = cbz(&[
        ("01.png", &BLUE_PNG),
        ("02.png", &RED_PNG),
        ("03.png", &RED_PNG),
    ]);
    std::fs::write(folder.join("book.cbz"), &replaced).unwrap();
    let response = page(0).bearer_auth(&admin).send().await.unwrap();
    assert_error(response, StatusCode::CONFLICT, "content_changed").await;
    server
        .wait_until(|| async {
            let pages = body(
                server
                    .get(&format!("/api/v1/publications/{id}/pages"))
                    .bearer_auth(&admin)
                    .send()
                    .await
                    .unwrap(),
            )
            .await;
            pages["items"].as_array().is_some_and(|p| p.len() == 3)
        })
        .await;
    let response = page(0)
        .bearer_auth(&admin)
        .header(IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the old ETag no longer matches"
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), BLUE_PNG);
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_land_inside_the_library_folder() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let folder = server.folder("uploads", &[]);
    let library = server.folder_library(&admin, "Uploads", &folder).await;
    let book = cbz(&[("01.png", &RED_PNG)]);
    let file = |path: &str| server.put(&format!("/api/v1/libraries/{library}/files/{path}"));

    // `..` arrives percent-encoded, as a client would have to send it.
    let response = file("..%2Fescape.cbz")
        .bearer_auth(&admin)
        .body(book.clone())
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;
    assert!(!server.data.path().join("escape.cbz").exists());

    let response = file("Series/one.cbz")
        .bearer_auth(&admin)
        .body(book.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let publication = body(response).await;
    assert_eq!(publication["format"], "cbz");
    assert_eq!(publication["pageCount"], 1);
    assert_eq!(
        std::fs::read(folder.join("Series/one.cbz")).unwrap(),
        book,
        "the upload is kept byte for byte"
    );
    let id = publication["id"].as_i64().unwrap();
    let response = server
        .get(&format!("/api/v1/publications/{id}/pages/0"))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), RED_PNG);

    // Nothing is ever replaced through an upload.
    let response = file("Series/one.cbz")
        .bearer_auth(&admin)
        .body(book.clone())
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::CONFLICT, "file_exists").await;

    // A body that isn't the format its name claims leaves nothing behind.
    let response = file("Series/two.cbz")
        .bearer_auth(&admin)
        .body("not an archive")
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "bad_request").await;
    let left: Vec<_> = std::fs::read_dir(folder.join("Series"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["one.cbz"]);

    // Readers can't upload.
    let reader = server.reader(&admin, "reader", &[library]).await;
    let response = file("Series/three.cbz")
        .bearer_auth(&reader)
        .body(book)
        .send()
        .await
        .unwrap();
    assert_error(response, StatusCode::FORBIDDEN, "forbidden").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn zipped_fictionbooks_are_books() {
    let server = Server::start().await;
    let admin = server.admin().await;
    let fb2 = br#"<?xml version="1.0" encoding="UTF-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
  <description><title-info><book-title>The Zipped Book</book-title></title-info></description>
  <body><section><p>Hello.</p></section></body>
</FictionBook>"#;
    let zipped = cbz(&[("The Zipped Book.fb2", fb2)]);
    let folder = server.folder(
        "books",
        &[
            ("zipped.fb2.zip", &zipped),
            ("plain.fb2", fb2),
            ("other.zip", &cbz(&[("notes.txt", b"x")])),
        ],
    );
    let library = server.folder_library(&admin, "Books", &folder).await;

    let mut publications = server.publications(&admin, library).await;
    publications.sort_by_key(|p| p["title"].as_str().unwrap_or_default().to_owned());
    // The plain and zipped books are read alike; a ZIP without a FictionBook isn't a book.
    assert_eq!(
        publications
            .iter()
            .map(|p| (p["title"].as_str().unwrap(), p["format"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        [("The Zipped Book", "fb2"), ("The Zipped Book", "fb2")],
        "{publications:?}"
    );
    for publication in &publications {
        let id = publication["id"].as_i64().unwrap();
        let response = server
            .get(&format!("/api/v1/publications/{id}/file"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response.headers()[CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = response.bytes().await.unwrap();
        if bytes.starts_with(b"PK") {
            assert_eq!(content_type, "application/x-zip-compressed-fb2");
            assert_eq!(bytes.as_ref(), zipped);
        } else {
            assert_eq!(content_type, "application/x-fictionbook+xml");
        }
    }
}
