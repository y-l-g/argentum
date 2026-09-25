//! Where uploaded bytes go: the `Uploader` seam, and the one place
//! the framework hands bytes to it.
//!
//! `FileUpload` binds a `String` column, the panel renders a file input, and
//! the form parser decodes the multipart body — but *where* the bytes live and
//! what path the record stores is the app's decision: an object store, a
//! directory on disk, a CDN. The framework owns everything up to the bytes and
//! nothing after them, so this module is deliberately small: a trait, the app
//! context value that carries it, and the call that runs it.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
};

use topcoat::context::{Cx, try_app_context};

use crate::schema::Schema;

/// Store one uploaded file and name the value a record stores.
///
/// Installed once per panel with [`Panel::uploads`](crate::Panel::uploads) —
/// the way `Db` is — and found on the app context wherever a `FileUpload`
/// stores, because an object store is an app-level dependency: threading it
/// through every `.for(..)` call site would put it in the schema declaration.
///
/// `filename` arrives already sanitized to a basename: no directory
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
    /// error ("`<Label>` could not be uploaded: `<reason>`"), so it must be
    /// something they can act on — never a filesystem path, a driver message,
    /// or anything else the deployment would rather not print. A rejection is
    /// user input going wrong, not infrastructure: it re-renders the form with
    /// the submitted values instead of failing the request.
    fn store(
        &self,
        filename: &str,
        bytes: &[u8],
    ) -> impl Future<Output = Result<String, String>> + Send;

    /// Whether `path` is a value this store produced and still holds.
    ///
    /// A form that re-renders with errors carries the path a just-finished
    /// store returned, so the file survives the next submit; the framework asks
    /// this before it re-uses that path, which keeps the value's origin in the
    /// store rather than in whatever the client sent: a client-typed path is
    /// stored only when the store itself vouches for it.
    ///
    /// The contract is **ownership, not bare existence**: answer `true` only
    /// for a path this store returned from [`store`](Self::store) and still
    /// resolves **inside its own root**. An existence check on a
    /// client-supplied path — `Path::exists`, a `HEAD` on any URL — would turn
    /// this into a path-traversal gate.
    ///
    /// The default answers `false`: a re-rendered form does not carry an
    /// upload, and the next submit fails the field's `required` rule (create)
    /// or keeps the record's stored file (edit).
    fn holds(&self, path: &str) -> impl Future<Output = bool> + Send {
        async move {
            let _ = path;
            false
        }
    }
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

/// Boxed future of [`DynUploader::holds`].
type HoldFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// Dyn-compatible view of [`Uploader`].
///
/// [`Uploader`] returns `impl Future` (the house style: no `async_trait`
/// dependency, no hand-boxed signatures for implementors), which is not
/// dyn-compatible. The panel holds whichever uploader the app installed
/// without becoming generic over it, so the calls it makes go through this
/// shim — the public trait stays the shape an app implements, and the box
/// stays here.
pub(crate) trait DynUploader: Send + Sync {
    fn store<'a>(&'a self, filename: &'a str, bytes: &'a [u8]) -> StoreFuture<'a>;
    fn holds<'a>(&'a self, path: &'a str) -> HoldFuture<'a>;
}

impl<U: Uploader> DynUploader for U {
    fn store<'a>(&'a self, filename: &'a str, bytes: &'a [u8]) -> StoreFuture<'a> {
        Box::pin(Uploader::store(self, filename, bytes))
    }

    fn holds<'a>(&'a self, path: &'a str) -> HoldFuture<'a> {
        Box::pin(Uploader::holds(self, path))
    }
}

/// Inline errors from a failed store, keyed by field name: the shape
/// [`Schema::validate`](crate::schema::Schema::validate) answers with, so the
/// handler merges the two without translating.
pub(crate) type UploadErrors = HashMap<String, Vec<String>>;

/// Whether this panel has an uploader installed.
///
/// The multipart parser asks before staging bytes: with no uploader they would
/// be buffered only to be dropped, and today's drain-and-discard is what keeps
/// a large upload off the heap for every app that never installs one.
pub(crate) fn installed(cx: &Cx) -> bool {
    try_app_context::<InstalledUploader>(cx).is_some()
}

/// The installed uploader, if this panel has one.
///
/// The concrete boxed trait object keeps its auto traits: dropping `Send +
/// Sync` from the reference is not a coercion the compiler performs here.
type DynUploaderRef<'a> = &'a (dyn DynUploader + Send + Sync);

fn installed_uploader<'a>(cx: &'a Cx) -> Option<DynUploaderRef<'a>> {
    try_app_context::<InstalledUploader>(cx).map(|installed| &*installed.0)
}

/// Whether the installed uploader still holds `path`.
///
/// `false` without an installed uploader: nothing stored the bytes, so nothing
/// can vouch for a path a re-rendered form carried.
pub(crate) async fn holds(cx: &Cx, path: &str) -> bool {
    match installed_uploader(cx) {
        Some(uploader) => uploader.holds(path).await,
        None => false,
    }
}

/// Run the installed uploader over the file parts this form submitted
/// returning `field_name -> inline errors` and the fields whose
/// value is now the uploader's answer.
///
/// For each declared [`FileUpload`](crate::schema::FileUpload) that carried
/// bytes, the returned path replaces the sanitized basename the parser put in
/// `values` — so the record fn sees the stored path and nothing else changes
/// about its contract. The second half of the answer is those field names: a
/// form that re-renders carries them so the next submit can keep the upload.
/// Without an installed uploader this is a no-op: the sanitized basename stays
/// and nothing is carried, because a client's filename is not a stored file.
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
) -> (UploadErrors, HashSet<String>) {
    let Some(uploader) = installed_uploader(cx) else {
        return (UploadErrors::new(), HashSet::new());
    };
    let mut errors = UploadErrors::new();
    let mut stored = HashSet::new();
    // Declared uploads only: a file part the schema does not declare is not a
    // field this form may write (the unknown-key allow-list answers for it).
    for (name, upload) in schema.file_uploads() {
        let Some(staged) = files.get(&name) else {
            continue;
        };
        match uploader.store(&staged.filename, &staged.bytes).await {
            Ok(path) => {
                values.insert(name.clone(), path);
                stored.insert(name);
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
    (errors, stored)
}
