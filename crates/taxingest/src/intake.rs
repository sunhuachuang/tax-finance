use std::fs;
use std::path::Path;

use chrono::Utc;
use sha2::{Digest, Sha256};
use taxcore::{Document, DocumentId, DocumentSource, DocumentStatus};
use taxstore::Store;

use crate::Result;

pub struct IncomingFile {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub source: DocumentSource,
    pub original_filename: Option<String>,
}

pub enum Intake {
    Stored(Document),
    /// The same bytes are already on file. The caller decides what a duplicate
    /// means — usually nothing, sometimes "the email and the photo were the
    /// same receipt".
    Duplicate(Document),
}

impl Intake {
    pub fn document(&self) -> &Document {
        match self {
            Intake::Stored(doc) | Intake::Duplicate(doc) => doc,
        }
    }
}

/// Store the bytes content-addressed under `data_dir` and register the
/// document as pending extraction. Idempotent on content: the same bytes
/// arriving twice — whatever the filename or source — come back as
/// [`Intake::Duplicate`] and nothing is written.
pub fn ingest(store: &mut Store, data_dir: &Path, file: IncomingFile) -> Result<Intake> {
    let sha256 = format!("{:x}", Sha256::digest(&file.bytes));
    if let Some(existing) = store.document_by_sha256(&sha256)? {
        return Ok(Intake::Duplicate(existing));
    }

    let stored_path = format!("docs/{}/{}.{}", &sha256[..2], sha256, extension(&file));
    let absolute = data_dir.join(&stored_path);
    fs::create_dir_all(absolute.parent().expect("stored_path has a parent"))?;
    fs::write(&absolute, &file.bytes)?;

    let document = Document {
        id: DocumentId::new(),
        sha256,
        source: file.source,
        mime: file.mime,
        byte_len: file.bytes.len() as u64,
        stored_path,
        original_filename: file.original_filename,
        received_at: Utc::now(),
        status: DocumentStatus::PendingExtraction,
    };
    store.insert_document(&document)?;
    Ok(Intake::Stored(document))
}

/// The bytes back out, for whoever is going to read them.
pub fn document_bytes(data_dir: &Path, document: &Document) -> Result<Vec<u8>> {
    Ok(fs::read(data_dir.join(&document.stored_path))?)
}

/// A display-friendly extension for the stored file. Cosmetic only — identity
/// is the hash and interpretation follows the mime type — so unknowns fall
/// back to `bin` rather than failing.
fn extension(file: &IncomingFile) -> String {
    if let Some(name) = &file.original_filename
        && let Some((_, ext)) = name.rsplit_once('.')
        && !ext.is_empty()
        && ext.len() <= 5
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return ext.to_ascii_lowercase();
    }
    match file.mime.as_str() {
        "application/pdf" => "pdf",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/heic" => "heic",
        "text/csv" => "csv",
        "text/plain" => "txt",
        _ => "bin",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use crate::testutil::{store, upload};

    use super::*;

    #[test]
    fn bytes_are_stored_content_addressed_and_read_back() {
        let mut store = store();
        let dir = tempfile::tempdir().unwrap();

        let intake = ingest(&mut store, dir.path(), upload(b"fake pdf bytes")).unwrap();
        let Intake::Stored(doc) = intake else {
            panic!("first ingest must store");
        };
        assert_eq!(doc.status, DocumentStatus::PendingExtraction);
        assert!(doc.stored_path.starts_with("docs/"));
        assert!(doc.stored_path.ends_with(".pdf"));
        assert_eq!(doc.byte_len, 14);

        assert_eq!(document_bytes(dir.path(), &doc).unwrap(), b"fake pdf bytes");
        assert_eq!(store.document(doc.id).unwrap().sha256, doc.sha256);
    }

    #[test]
    fn the_same_bytes_from_anywhere_are_one_document() {
        let mut store = store();
        let dir = tempfile::tempdir().unwrap();

        let first = ingest(&mut store, dir.path(), upload(b"same receipt")).unwrap();

        let mut photo = upload(b"same receipt");
        photo.mime = "image/jpeg".into();
        photo.original_filename = None;
        photo.source = taxcore::DocumentSource::Photo;
        let second = ingest(&mut store, dir.path(), photo).unwrap();

        let Intake::Duplicate(dup) = second else {
            panic!("identical bytes must dedup");
        };
        assert_eq!(dup.id, first.document().id);

        let different = ingest(&mut store, dir.path(), upload(b"another receipt")).unwrap();
        assert!(matches!(different, Intake::Stored(_)));
    }

    #[test]
    fn hostile_filenames_do_not_choose_the_extension() {
        let mut store = store();
        let dir = tempfile::tempdir().unwrap();

        let mut file = upload(b"bytes");
        file.original_filename = Some("../../etc/passwd".into());
        let intake = ingest(&mut store, dir.path(), file).unwrap();
        // "passwd" is 6 chars and ".." is not alphanumeric; falls back to mime.
        assert!(intake.document().stored_path.ends_with(".pdf"));
    }
}
