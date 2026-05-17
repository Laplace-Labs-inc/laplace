use thiserror::Error;

pub type LaplaceResult<T> = Result<T, LaplaceError>;

#[derive(Debug, Error)]
pub enum LaplaceError {
    #[error("I/O error accessing `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Rust parse error in `{file}`: {msg}")]
    RustParse { file: String, msg: String },

    #[allow(dead_code)]
    #[error("Markdown parse error in `{file}`: {msg}")]
    MdParse { file: String, msg: String },

    #[allow(dead_code)]
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Failed to create directory `{path}`: {source}")]
    DirCreate {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[allow(dead_code)]
    #[error("database error: {msg}")]
    Db { msg: String },
}
