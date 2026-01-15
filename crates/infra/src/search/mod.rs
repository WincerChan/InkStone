pub mod query_parser;
pub mod tantivy_index;

pub use query_parser::{parse_query, QueryParseError};
pub use tantivy_index::{SearchIndex, SearchIndexError, SearchIndexStats};

#[derive(Debug, Clone, Copy)]
pub enum SearchSort {
    Relevance,
    Latest,
}

impl Default for SearchSort {
    fn default() -> Self {
        Self::Relevance
    }
}
