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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStrategy {
    Jieba,
    Lindera,
    Ngram,
}

impl SearchStrategy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "jieba" => Some(Self::Jieba),
            "lindera" => Some(Self::Lindera),
            "ngram" => Some(Self::Ngram),
            _ => None,
        }
    }

    pub fn as_dir_name(self) -> &'static str {
        match self {
            Self::Jieba => "jieba",
            Self::Lindera => "lindera",
            Self::Ngram => "ngram",
        }
    }
}
