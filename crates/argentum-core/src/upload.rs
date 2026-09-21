//! Where uploaded bytes go (GH #188): the `Uploader` seam, and the one place
//! the framework hands bytes to it.
//!
//! `FileUpload` binds a `String` column, the panel renders a file input, and
//! the form parser decodes the multipart body — but *where* the bytes live and
//! what path the record stores is the app's decision: an object store, a
//! directory on disk, a CDN. The framework owns everything up to the bytes and
//! nothing after them, so this module is deliberately small: a trait, the app
//! context value that carries it, and the call that runs it.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use topcoat::context::{Cx, try_app_context};

use crate::schema::Schema;

/// Store one uploaded file and name the value a record stores (GH #188).
///
/// Installed once per panel with [`Panel::uploads`](crate::Panel::uploads) —
/// the way `Db` is — and found on the app context wherever a `FileUpload`
/// stores, because an object store is an app-level dependency: threading it
/// through every `.for(..)` call site would put it in the schema declaration.
///
/// `filename` arrives already sanitized to a basename (GH #90): no directory
/// components, no control characters, capped at 255 bytes, and never empty (an
/// empty filename is "no file chosen", which never reaches an uploader).
/// `bytes` are the part's content, bounded by the form-body cap (10 MiB,
/// `MAX_FORM_BYTES`). Everything else is the
/// app's business: generating a collision-free name, choosing a directory or
/// bucket, and deciding what the returned path means — the framework stores it
/// verbatim and renders it as the stored value.
pub trait Uploader: Send + Sync + 'static {
    /// Store `bytes` and return the value to store for this field.
    ///
    /// The `Err` string is rendered to the user inside the field's inline
    /// error ("<Label> could not be uploaded: <reason>"), so it must be
    /// something they can act on — never a filesystem path, a driver message,
    /// or anything else the deployment would rather not print. A rejection is
    /// user input going wrong, not infrastructure: it re-renders the form with
    /// the submitted values instead of failing the request.
    fn store(
        &self,
        filename: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<String, String>> + Send;
}

/// A file part a form submitted: the sanitized basename and its bytes.
///
/// Staged by the multipart parser (`panel::forms`) only when an [`Uploader`] is
/// installed. Without one the bytes are still drained and dropped, exactly as
/// they were before GH #188, so an app that installs no uploader keeps the
/// constant-memory path it had.
#[derive(Debug, Clone)]
pub(crate) struct StagedUpload {
    pub(crate) filename: String,
    pub(crate) bytes: Vec<u8>,
}

/// The one uploader a panel was built with, on the app context the way `Db` is.
pub(crate) struct InstalledUploader(Box<dyn DynUploader + Send + Sync>);

impl InstalledUploader {
    pub(crate) fn new(uploader: impl Uploader) -> Self {
        Self(Box::new(uploader))
    }
}

/// Boxed future of [`DynUploader::store`].
type StoreFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

/// Dyn-compatible view of [`Uploader`].
///
/// [`Uploader`] returns `impl Future` (the house style: no `async_trait`
/// dependency, no hand-boxed signatures for implementors), which is not
/// dyn-compatible. The panel holds whichever uploader the app installed
/// without becoming generic over it, so the single call it makes goes through
/// this shim — the public trait stays the shape an app implements, and the
/// box stays here.
pub(crate) trait DynUploader: Send + Sync {
    fn store<'a>(&'a self, filename: &'a str, bytes: &'a [u8]) -> StoreFuture<'a>;
}

impl<U: Uploader> DynUploader for U {
    fn store<'a>(&'a self, filename: &'a str, bytes: &'a [u8]) -> StoreFuture<'a> {
        Box::pin(Uploader::store(self, filename, bytes))
    }
}

/// Inline errors from a failed store, keyed by field name: the shape
/// [`Schema::validate`](crate::schema::Schema::validate) answers with, so the
/// handler merges the two without translating.
pub(crate) type UploadErrors = HashMap<String, Vec<String>>;

/// Whether this panel has an uploader installed (GH #188).
///
/// The multipart parser asks before staging bytes: with no uploader they would
/// be buffered only to be dropped, and today's drain-and-discard is what keeps
/// a large upload off the heap for every app that never installs one.
pub(crate) fn installed(cx: &Cx) -> bool {
    try_app_context::<InstalledUploader>(cx).is_some()
}

/// Run the installed uploader over the file parts this form submitted
/// (GH #188), returning `field_name -> inline errors`.
///
/// For each declared [`FileUpload`](crate::schema::FileUpload) that carried
/// bytes, the returned path replaces the sanitized basename the parser put in
/// `values` — so the record fn sees the stored path and nothing else changes
/// about its contract. Without an installed uploader this is a no-op: the
/// basename stays, which is exactly the pre-#188 behaviour an app that never
/// installs one must keep.
///
/// A failed store becomes an inline field error and **drops the submitted
/// value**, because there is no path to store: nothing was written, and the
/// client's filename is not a stored file. Dropping it is what lets the caller
/// re-render honestly — run this *before* the edit handler's untouched-value
/// backfill, which then restores the path that is actually stored, while a
/// create renders the field empty beside the reason. Files stored earlier in
/// the same call are *not* rolled back — their paths never reach the record, so
/// they are unreferenced rather than wrong; a store with a real write cost
/// wants its own janitor, which is the app's call, not the framework's.
///
/// Call this outside the write transaction: an upload is a side effect in
/// another system, and a rolled-back transaction must not have to undo it.
pub(crate) async fn store_uploads(
    cx: &Cx,
    schema: &Schema,
    files: &HashMap<String, StagedUpload>,
    values: &mut HashMap<String, String>,
) -> UploadErrors {
    let Some(uploader) = try_app_context::<InstalledUploader>(cx).map(|installed| &*installed.0)
    else {
        return UploadErrors::new();
    };
    let mut errors = UploadErrors::new();
    // Declared uploads only: a file part the schema does not declare is not a
    // field this form may write (the unknown-key allow-list answers for it).
    for (name, upload) in schema.file_uploads() {
        let Some(staged) = files.get(&name) else {
            continue;
        };
        match uploader.store(&staged.filename, &staged.bytes).await {
            Ok(path) => {
                values.insert(name, path);
            }
            Err(reason) => {
                values.remove(&name);
                errors.insert(
                    name,
                    vec![format!(
                        "{} could not be uploaded: {reason}",
                        upload.label_str()
                    )],
                );
            }
        }
    }
    errors
}
