pub mod error;
pub mod types;

pub use error::Error;
pub use types::{Edge, EdgeType, File, GraphMeta, IndexState, Lang, Symbol, SymbolKind};

pub type Result<T> = std::result::Result<T, Error>;
