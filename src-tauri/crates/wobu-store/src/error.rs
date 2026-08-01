use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0} is not a Wobu project (no project.json)")]
    NotAProject(PathBuf),

    #[error("{0} already exists")]
    AlreadyExists(PathBuf),

    #[error("choose a new export folder path, not {0}")]
    InvalidExportDestination(PathBuf),

    #[error("the static wiki destination must be outside the project folder: {0}")]
    ExportInsideProject(PathBuf),

    #[error(
        "resolve {corrupt} broken node file(s) and {conflicts} conflict(s) before exporting"
    )]
    ExportBlocked { corrupt: usize, conflicts: usize },

    #[error(
        "this project was written by a newer version of Wobu (schema {found}, this build understands {supported})"
    )]
    SchemaTooNew { found: u32, supported: u32 },

    #[error("no node with id {0}")]
    NoSuchNode(String),

    /// An update or removal named an influence edge the node no longer has.
    #[error("no {role} link from that node to node {target}")]
    NoSuchNodeLink { target: String, role: String },

    /// The kind registry is the vocabulary for the Relations picker. Keeping
    /// the same guard at the store boundary prevents a stale or hand-written
    /// command from adding an edge the UI could never subsequently offer.
    #[error("{kind} nodes do not support {role} links")]
    InvalidNodeLinkRole { kind: String, role: String },

    /// A link named an asset the project does not have.
    ///
    /// Refused rather than stored, because the id in a link is derived from a
    /// file's hash and nothing else: an id that matches no file matches no file
    /// on any machine, forever. Writing it into frontmatter would put a
    /// permanent dangling reference on a share for everyone to trip over — the
    /// exact failure deriving asset ids was meant to make impossible.
    #[error("no asset with id {0} in this project")]
    NoSuchAsset(String),

    /// An update or an unlink named a link that is not on the node.
    ///
    /// Almost always a UI holding a reference the user removed elsewhere — or,
    /// on a share, one a collaborator removed. Saying so beats writing the link
    /// back into existence to satisfy the request.
    #[error("no {role} link from that node to asset {asset}")]
    NoSuchAssetLink { asset: String, role: String },

    /// Destructive asset deletion is only available for true orphans.
    ///
    /// Checked at the store boundary rather than trusted to the library UI: a
    /// collaborator can attach the image after the confirmation sheet opens,
    /// and deleting then would leave their frontmatter pointing at nothing.
    #[error("asset {asset} is still used by {nodes} node(s); detach it and clear any covers first")]
    AssetInUse { asset: String, nodes: usize },

    /// A resolution named a path that is not a conflict sibling.
    ///
    /// Its own variant rather than a generic invalid-argument, because
    /// `resolve_conflict` is the one function in Wobu that deletes a file the
    /// user did not ask to delete by name, and the check that keeps it pointed
    /// at conflict siblings deserves to fail loudly rather than blend in.
    #[error("{0} is not a conflict sibling")]
    NotAConflict(PathBuf),

    #[error("malformed node file {path}: {reason}")]
    Malformed { path: PathBuf, reason: String },

    #[error("malformed generation record {path}: {reason}")]
    MalformedGeneration { path: PathBuf, reason: String },

    #[error("{0} is missing its YAML frontmatter")]
    MissingFrontmatter(PathBuf),

    /// An import whose bytes no supported header parser recognised.
    ///
    /// Its own variant rather than a generic invalid-argument, because it is
    /// the one asset failure a user can act on: they dropped a PDF, or a `.psd`
    /// with a `.png` name, and the answer is to convert it. Naming the formats
    /// in the message is the whole value of the error.
    #[error("that file is not an image Wobu can read (PNG, JPEG, GIF and WebP are supported)")]
    NotAnImage,

    /// An import of an animation — an animated GIF, an APNG, an animated WebP.
    ///
    /// Its own variant rather than folding into [`Error::NotAnImage`], because
    /// the two need opposite advice: that one means "convert it", and this one
    /// means "export the frame you meant". Telling someone their animated GIF
    /// is not an image Wobu can read would be both wrong and unactionable —
    /// they can see it perfectly well in their file browser.
    ///
    /// Refusing rather than silently taking frame one is the deliberate half.
    /// A reference image is one picture; picking a frame on the user's behalf
    /// makes the asset depend on a choice nothing on screen records, and
    /// re-encoding a single frame out would make the blob's hash depend on our
    /// encoder rather than on their file. See `assets`' module docs.
    #[error(
        "that image holds more than one frame — export the single frame you want and import that"
    )]
    AnimatedImage,

    #[error("that mesh is not a complete binary glTF 2.0 file")]
    NotAMesh,

    /// A blob already in the library whose pixels will not come back out.
    ///
    /// Distinct from [`Error::NotAnImage`], which is a file refused at the
    /// door, and the difference is what the caller does next. This one is
    /// reached only by something that decodes — a thumbnail, and later a
    /// provider payload — and the usual cause is a blob a sync client has not
    /// finished copying: the header parsed, which is how it got indexed, and
    /// the pixel data stops early. That is a *transient* state, so the answer
    /// is to leave the thumbnail unmade and ask again later rather than to
    /// write a broken one or drop the asset.
    #[error("{path} could not be decoded: {reason}")]
    Undecodable { path: PathBuf, reason: String },

    #[error("the project folder is read-only")]
    ReadOnly,

    /// The folder was reachable when the project was opened and is not now.
    ///
    /// Distinct from [`Error::NotAProject`], which is a folder the user picked
    /// that never was one. This is a share that went away underneath an open
    /// session, and the difference matters: the index is still a complete copy
    /// of the world, so the right response is to hold on and wait rather than
    /// to close the project.
    #[error("the project folder is not reachable — the share may be unmounted")]
    Disconnected,

    #[error("no project is open")]
    NoProjectOpen,

    #[error("the source and destination are the same project")]
    TransferSameProject,

    /// The user stopped a long scan.
    ///
    /// Not a failure, and deliberately its own variant: nothing was written and
    /// the folder is exactly as it was, so the UI must not offer to retry
    /// something the user just asked to stop, nor report it as trouble.
    #[error("cancelled")]
    Cancelled,

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_norway::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("index error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Core(#[from] wobu_core::Error),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io { path: path.into(), source }
    }
}
