use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use inkstone_core::domain::search::{SearchDocument, SearchHit, SearchQuery, SearchResult};
use std::ops::Bound;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, EmptyQuery, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, SchemaBuilder, TextFieldIndexing, TextOptions, Value, FAST,
    STORED, STRING,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{
    LowerCaser, NgramTokenizer, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer, Token,
    TokenStream, Tokenizer,
};
use tantivy::{DocAddress, Index, IndexReader, Order, ReloadPolicy, Score, TantivyDocument, Term};
use thiserror::Error;

use super::SearchSort;

const TOKENIZER_CJK_BG: &str = "cjk_bg";
const TOKENIZER_CJK_UG: &str = "cjk_ug";
const TOKENIZER_LATIN: &str = "latin";

#[derive(Debug, Error)]
pub enum SearchIndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("missing tokenizer: {0}")]
    MissingTokenizer(&'static str),
    #[error("missing field in schema: {0}")]
    MissingField(&'static str),
    #[error("missing stored value: {0}")]
    MissingValue(&'static str),
    #[error("invalid stored timestamp: {0}")]
    InvalidTimestamp(&'static str),
}

#[derive(Debug, Clone)]
struct SearchFields {
    id: Field,
    title: Field,
    subtitle: Field,
    content: Field,
    title_cjk_bg: Option<Field>,
    subtitle_cjk_bg: Option<Field>,
    content_cjk_bg: Option<Field>,
    title_cjk_ug: Option<Field>,
    subtitle_cjk_ug: Option<Field>,
    content_cjk_ug: Option<Field>,
    title_latin: Option<Field>,
    subtitle_latin: Option<Field>,
    content_latin: Option<Field>,
    url: Field,
    tags: Field,
    category: Field,
    published: Field,
    updated: Field,
    checksum: Field,
}

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    fields: SearchFields,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexStats {
    pub num_docs: u64,
    pub num_segments: usize,
}

impl SearchIndex {
    pub fn open_or_create(
        path: impl AsRef<Path>,
    ) -> Result<Self, SearchIndexError> {
        let dir = path.as_ref();
        std::fs::create_dir_all(dir)?;

        let schema = build_schema();
        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir)?
        } else {
            Index::create_in_dir(dir, schema)?
        };
        register_tokenizers(&index)?;
        let schema = index.schema();
        let fields = SearchFields::from_schema(&schema)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    pub fn search(
        &self,
        query: &SearchQuery,
        limit: usize,
        offset: usize,
        sort: SearchSort,
    ) -> Result<SearchResult, SearchIndexError> {
        let searcher = self.reader.searcher();
        let built_query = build_query(&self.index, &self.fields, query)?;
        let highlight_snippets = build_highlight_snippets(query, &self.fields)?;
        let (title_snippet, subtitle_snippet, content_snippet) = match highlight_snippets.as_ref() {
            Some(snippets) => (
                Some(&snippets.title),
                Some(&snippets.subtitle),
                Some(&snippets.content),
            ),
            None => (None, None, None),
        };
        let total = searcher.search(&built_query.query, &Count)?;
        let docs: Vec<DocAddress> = match sort {
            SearchSort::Relevance => searcher
                .search(
                    &built_query.query,
                    &TopDocs::with_limit(limit.saturating_add(offset)),
                )?
                .into_iter()
                .map(|(_, address)| address)
                .collect(),
            SearchSort::Latest => {
                let collector = TopDocs::with_limit(limit.saturating_add(offset))
                    .order_by_fast_field::<i64>("updated", Order::Desc);
                searcher
                    .search(&built_query.query, &collector)?
                    .into_iter()
                    .map(|(_, address)| address)
                    .collect()
            }
        };

        let mut hits = Vec::new();
        for address in docs.into_iter().skip(offset).take(limit) {
            let doc: TantivyDocument = searcher.doc(address)?;
            hits.push(self.document_to_hit(
                &doc,
                title_snippet,
                subtitle_snippet,
                content_snippet,
            )?);
        }

        Ok(SearchResult { total, hits })
    }

    pub fn stats(&self) -> SearchIndexStats {
        let searcher = self.reader.searcher();
        SearchIndexStats {
            num_docs: searcher.num_docs(),
            num_segments: searcher.segment_readers().len(),
        }
    }

    pub fn get_checksum(&self, id: &str) -> Result<Option<String>, SearchIndexError> {
        let searcher = self.reader.searcher();
        let term = Term::from_field_text(self.fields.id, id);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let docs = searcher.search(&query, &TopDocs::with_limit(1))?;
        let Some((_, address)) = docs.into_iter().next() else {
            return Ok(None);
        };
        let doc: TantivyDocument = searcher.doc(address)?;
        let checksum = get_string(&doc, self.fields.checksum)
            .ok_or(SearchIndexError::MissingValue("checksum"))?;
        Ok(Some(checksum))
    }

    pub fn upsert_documents(&self, documents: &[SearchDocument]) -> Result<(), SearchIndexError> {
        let mut writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        for doc in documents {
            writer.delete_term(Term::from_field_text(self.fields.id, &doc.id));
            writer.add_document(self.domain_to_document(doc))?;
        }
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn delete_all(&self) -> Result<(), SearchIndexError> {
        let mut writer = self.index.writer::<TantivyDocument>(50_000_000)?;
        writer.delete_all_documents()?;
        writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    fn domain_to_document(&self, doc: &SearchDocument) -> TantivyDocument {
        let mut document = TantivyDocument::default();
        document.add_text(self.fields.id, &doc.id);
        document.add_text(self.fields.title, &doc.title);
        if let Some(subtitle) = &doc.subtitle {
            document.add_text(self.fields.subtitle, subtitle);
        }
        document.add_text(self.fields.content, &doc.content);
        document.add_text(self.fields.url, &doc.url);
        for tag in &doc.tags {
            document.add_text(self.fields.tags, tag);
        }
        if let Some(category) = &doc.category {
            document.add_text(self.fields.category, category);
        }
        document.add_i64(self.fields.published, doc.published_at.timestamp());
        document.add_i64(self.fields.updated, doc.updated_at.timestamp());
        document.add_text(self.fields.checksum, &doc.checksum);
        if self.fields.title_cjk_bg.is_some() || self.fields.title_cjk_ug.is_some() {
            let normalized = normalize_cjk(&doc.title);
            if let Some(field) = self.fields.title_cjk_bg {
                document.add_text(field, &normalized);
            }
            if let Some(field) = self.fields.title_cjk_ug {
                document.add_text(field, &normalized);
            }
        }
        if self.fields.subtitle_cjk_bg.is_some() || self.fields.subtitle_cjk_ug.is_some() {
            let normalized = doc.subtitle.as_deref().map(normalize_cjk).unwrap_or_default();
            if let Some(field) = self.fields.subtitle_cjk_bg {
                document.add_text(field, &normalized);
            }
            if let Some(field) = self.fields.subtitle_cjk_ug {
                document.add_text(field, &normalized);
            }
        }
        if self.fields.content_cjk_bg.is_some() || self.fields.content_cjk_ug.is_some() {
            let normalized = normalize_cjk(&doc.content);
            if let Some(field) = self.fields.content_cjk_bg {
                document.add_text(field, &normalized);
            }
            if let Some(field) = self.fields.content_cjk_ug {
                document.add_text(field, &normalized);
            }
        }
        if let Some(field) = self.fields.title_latin {
            let normalized = normalize_latin(&doc.title);
            document.add_text(field, &normalized);
        }
        if let Some(field) = self.fields.subtitle_latin {
            let normalized = doc.subtitle.as_deref().map(normalize_latin).unwrap_or_default();
            document.add_text(field, &normalized);
        }
        if let Some(field) = self.fields.content_latin {
            let normalized = normalize_latin(&doc.content);
            document.add_text(field, &normalized);
        }
        document
    }

    fn document_to_hit(
        &self,
        doc: &TantivyDocument,
        title_snippet: Option<&SnippetGenerator>,
        subtitle_snippet: Option<&SnippetGenerator>,
        content_snippet: Option<&SnippetGenerator>,
    ) -> Result<SearchHit, SearchIndexError> {
        let title = get_string(doc, self.fields.title).ok_or(SearchIndexError::MissingValue("title"))?;
        let url = get_string(doc, self.fields.url).ok_or(SearchIndexError::MissingValue("url"))?;
        let id = url.clone();
        let tags = get_strings(doc, self.fields.tags);
        let category = get_string(doc, self.fields.category);
        let published = get_i64(doc, self.fields.published)
            .ok_or(SearchIndexError::MissingValue("published"))?;
        let updated = get_i64(doc, self.fields.updated)
            .ok_or(SearchIndexError::MissingValue("updated"))?;

        Ok(SearchHit {
            id,
            title: snippet_or_excerpt(title_snippet, doc, self.fields.title, 120)
                .unwrap_or(title),
            subtitle: snippet_html(subtitle_snippet, doc),
            content: snippet_or_excerpt(content_snippet, doc, self.fields.content, 120),
            url,
            tags,
            category,
            published_at: timestamp_to_datetime(published, "published")?,
            updated_at: timestamp_to_datetime(updated, "updated")?,
        })
    }
}

#[cfg(test)]
mod stats_tests {
    use super::SearchIndex;
    use inkstone_core::domain::search::SearchDocument;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{nanos}"))
    }

    #[test]
    fn stats_reflect_indexed_docs() {
        let dir = temp_dir("inkstone-search-stats");
        fs::create_dir_all(&dir).unwrap();
        let index = SearchIndex::open_or_create(&dir).unwrap();
        let doc = SearchDocument {
            id: "doc-1".to_string(),
            title: "Hello".to_string(),
            subtitle: None,
            content: "World".to_string(),
            url: "https://example.com/posts/hello".to_string(),
            tags: vec![],
            category: None,
            published_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            checksum: "checksum".to_string(),
        };
        index.upsert_documents(&[doc]).unwrap();
        let stats = index.stats();
        assert_eq!(stats.num_docs, 1);
        let _ = fs::remove_dir_all(&dir);
    }
}

impl SearchFields {
    fn from_schema(schema: &Schema) -> Result<Self, SearchIndexError> {
        Ok(Self {
            id: schema
                .get_field("id")
                .map_err(|_| SearchIndexError::MissingField("id"))?,
            title: schema
                .get_field("title")
                .map_err(|_| SearchIndexError::MissingField("title"))?,
            subtitle: schema
                .get_field("subtitle")
                .map_err(|_| SearchIndexError::MissingField("subtitle"))?,
            content: schema
                .get_field("content")
                .map_err(|_| SearchIndexError::MissingField("content"))?,
            title_cjk_bg: schema.get_field("title_cjk_bg").ok(),
            subtitle_cjk_bg: schema.get_field("subtitle_cjk_bg").ok(),
            content_cjk_bg: schema.get_field("content_cjk_bg").ok(),
            title_cjk_ug: schema.get_field("title_cjk_ug").ok(),
            subtitle_cjk_ug: schema.get_field("subtitle_cjk_ug").ok(),
            content_cjk_ug: schema.get_field("content_cjk_ug").ok(),
            title_latin: schema.get_field("title_latin").ok(),
            subtitle_latin: schema.get_field("subtitle_latin").ok(),
            content_latin: schema.get_field("content_latin").ok(),
            url: schema
                .get_field("url")
                .map_err(|_| SearchIndexError::MissingField("url"))?,
            tags: schema
                .get_field("tags")
                .map_err(|_| SearchIndexError::MissingField("tags"))?,
            category: schema
                .get_field("category")
                .map_err(|_| SearchIndexError::MissingField("category"))?,
            published: schema
                .get_field("published")
                .map_err(|_| SearchIndexError::MissingField("published"))?,
            updated: schema
                .get_field("updated")
                .map_err(|_| SearchIndexError::MissingField("updated"))?,
            checksum: schema
                .get_field("checksum")
                .map_err(|_| SearchIndexError::MissingField("checksum"))?,
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = SchemaBuilder::default();
    builder.add_text_field("id", STRING | STORED);
    let (title_opts, subtitle_opts, content_opts) = (
        stored_text_options(),
        stored_text_options(),
        stored_text_options(),
    );
    builder.add_text_field("title", title_opts);
    builder.add_text_field("subtitle", subtitle_opts);
    builder.add_text_field("content", content_opts);
    builder.add_text_field("title_cjk_bg", cjk_bigram_text_options(false, false));
    builder.add_text_field("subtitle_cjk_bg", cjk_bigram_text_options(false, false));
    builder.add_text_field("content_cjk_bg", cjk_bigram_text_options(false, false));
    builder.add_text_field("title_cjk_ug", cjk_unigram_text_options(false, false));
    builder.add_text_field("subtitle_cjk_ug", cjk_unigram_text_options(false, false));
    builder.add_text_field("content_cjk_ug", cjk_unigram_text_options(false, false));
    builder.add_text_field("title_latin", latin_text_options(false, false));
    builder.add_text_field("subtitle_latin", latin_text_options(false, false));
    builder.add_text_field("content_latin", latin_text_options(false, false));
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("tags", STRING | STORED);
    builder.add_text_field("category", STRING | STORED);
    builder.add_i64_field("published", STORED | FAST);
    builder.add_i64_field("updated", STORED | FAST);
    builder.add_text_field("checksum", STRING | STORED);
    builder.build()
}

fn stored_text_options() -> TextOptions {
    TextOptions::default().set_stored()
}

fn cjk_bigram_text_options(stored: bool, with_positions: bool) -> TextOptions {
    let option = if with_positions {
        IndexRecordOption::WithFreqsAndPositions
    } else {
        IndexRecordOption::WithFreqs
    };
    text_options(TOKENIZER_CJK_BG, stored, option)
}

fn cjk_unigram_text_options(stored: bool, with_positions: bool) -> TextOptions {
    let option = if with_positions {
        IndexRecordOption::WithFreqsAndPositions
    } else {
        IndexRecordOption::WithFreqs
    };
    text_options(TOKENIZER_CJK_UG, stored, option)
}

fn latin_text_options(stored: bool, with_positions: bool) -> TextOptions {
    let option = if with_positions {
        IndexRecordOption::WithFreqsAndPositions
    } else {
        IndexRecordOption::WithFreqs
    };
    text_options(TOKENIZER_LATIN, stored, option)
}

fn text_options(
    tokenizer: &'static str,
    stored: bool,
    index_option: IndexRecordOption,
) -> TextOptions {
    let indexing = TextFieldIndexing::default()
        .set_tokenizer(tokenizer)
        .set_index_option(index_option);
    let options = TextOptions::default().set_indexing_options(indexing);
    if stored {
        options.set_stored()
    } else {
        options
    }
}

fn register_tokenizers(index: &Index) -> Result<(), SearchIndexError> {
    let analyzer = build_cjk_bigram_analyzer()?;
    index.tokenizers().register(TOKENIZER_CJK_BG, analyzer);
    let analyzer = build_cjk_unigram_analyzer()?;
    index.tokenizers().register(TOKENIZER_CJK_UG, analyzer);
    let analyzer = build_latin_analyzer();
    index.tokenizers().register(TOKENIZER_LATIN, analyzer);
    Ok(())
}

fn build_cjk_bigram_analyzer() -> Result<TextAnalyzer, SearchIndexError> {
    build_cjk_ngram_analyzer(2, 2)
}

fn build_cjk_unigram_analyzer() -> Result<TextAnalyzer, SearchIndexError> {
    build_cjk_ngram_analyzer(1, 1)
}

fn build_cjk_ngram_analyzer(
    min_gram: usize,
    max_gram: usize,
) -> Result<TextAnalyzer, SearchIndexError> {
    let tokenizer = NgramTokenizer::all_ngrams(min_gram, max_gram)?;
    Ok(TextAnalyzer::builder(tokenizer)
        .filter(LowerCaser)
        .build())
}

fn build_latin_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build()
}

fn build_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<BuiltQuery, SearchIndexError> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    let search_keyword_query = build_keyword_query(index, fields, query)?;
    if let Some(keyword_query) = search_keyword_query.as_ref() {
        clauses.push((Occur::Must, keyword_query.box_clone()));
    }

    if !query.tags.is_empty() {
        for tag in &query.tags {
            let term = Term::from_field_text(fields.tags, tag);
            let tag_query = TermQuery::new(term, IndexRecordOption::Basic);
            clauses.push((Occur::Must, Box::new(tag_query)));
        }
    }

    if let Some(category) = &query.category {
        let term = Term::from_field_text(fields.category, category);
        let category_query = TermQuery::new(term, IndexRecordOption::Basic);
        clauses.push((Occur::Must, Box::new(category_query)));
    }

    if let Some(range) = &query.range {
        let (start, end) = range.to_timestamp_bounds();
        if start.is_some() || end.is_some() {
            let published_query = build_range_query(fields.published, start, end);
            let updated_query = build_range_query(fields.updated, start, end);
            let range_query = BooleanQuery::from(vec![
                (Occur::Should, published_query),
                (Occur::Should, updated_query),
            ]);
            clauses.push((Occur::Must, Box::new(range_query)));
        }
    }

    let compiled_query: Box<dyn Query> = if clauses.is_empty() {
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::from(clauses))
    };
    Ok(BuiltQuery {
        query: compiled_query,
    })
}

struct BuiltQuery {
    query: Box<dyn Query>,
}

fn build_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    if query.keywords.is_empty() {
        return Ok(None);
    }
    build_ngram_keyword_query(index, fields, query)
}

fn build_ngram_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    let keyword_text = query.keywords.join(" ");
    let trimmed = keyword_text.trim();
    if trimmed.is_empty() {
        return Ok(Some(Box::new(EmptyQuery)));
    }
    let has_non_single_cjk_keyword = query.keywords.iter().any(|keyword| {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return false;
        }
        let cjk_text = normalize_cjk(keyword);
        let latin_raw = extract_latin_tokens(keyword);
        let is_single_cjk_only = cjk_text.chars().count() == 1 && latin_raw.is_empty();
        !is_single_cjk_only
    });

    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for keyword in &query.keywords {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            continue;
        }
        let mut keyword_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        let cjk_text = normalize_cjk(keyword);
        let latin_raw = extract_latin_tokens(keyword);
        let is_single_cjk_only = cjk_text.chars().count() == 1 && latin_raw.is_empty();
        if !cjk_text.is_empty() {
            let cjk_len = cjk_text.chars().count();
            let (
                tokenizer,
                title_field,
                subtitle_field,
                content_field,
                title_name,
                subtitle_name,
                content_name,
            ) = if cjk_len >= 2 {
                (
                    TOKENIZER_CJK_BG,
                    fields.title_cjk_bg,
                    fields.subtitle_cjk_bg,
                    fields.content_cjk_bg,
                    "title_cjk_bg",
                    "subtitle_cjk_bg",
                    "content_cjk_bg",
                )
            } else {
                (
                    TOKENIZER_CJK_UG,
                    fields.title_cjk_ug,
                    fields.subtitle_cjk_ug,
                    fields.content_cjk_ug,
                    "title_cjk_ug",
                    "subtitle_cjk_ug",
                    "content_cjk_ug",
                )
            };
            let mut analyzer = index
                .tokenizers()
                .get(tokenizer)
                .ok_or(SearchIndexError::MissingTokenizer(tokenizer))?;
            let tokens = dedup_terms(tokenize_terms(&mut analyzer, &cjk_text));
            if !tokens.is_empty() {
                let title = require_field(title_field, title_name)?;
                let subtitle = require_field(subtitle_field, subtitle_name)?;
                let content = require_field(content_field, content_name)?;
                if let Some(query) = build_ngram_tokens_query(title, subtitle, content, &tokens) {
                    keyword_clauses.push((Occur::Must, query));
                }
            }
        }

        if !latin_raw.is_empty() {
            let latin_text = latin_raw.join(" ");
            let mut analyzer = index
                .tokenizers()
                .get(TOKENIZER_LATIN)
                .ok_or(SearchIndexError::MissingTokenizer(TOKENIZER_LATIN))?;
            let tokens = dedup_terms(tokenize_terms(&mut analyzer, &latin_text));
            if !tokens.is_empty() {
                let title = require_field(fields.title_latin, "title_latin")?;
                let subtitle = require_field(fields.subtitle_latin, "subtitle_latin")?;
                let content = require_field(fields.content_latin, "content_latin")?;
                if let Some(query) = build_ngram_tokens_query(title, subtitle, content, &tokens) {
                    keyword_clauses.push((Occur::Must, query));
                }
            }
        }

        let keyword_query = if keyword_clauses.is_empty() {
            None
        } else if keyword_clauses.len() == 1 {
            Some(keyword_clauses.remove(0).1)
        } else {
            Some(Box::new(BooleanQuery::new(keyword_clauses)) as Box<dyn Query>)
        };
        if let Some(keyword_query) = keyword_query {
            let occur = if is_single_cjk_only && has_non_single_cjk_keyword {
                Occur::Should
            } else {
                Occur::Must
            };
            clauses.push((occur, keyword_query));
        }

        let tag_query = TermQuery::new(
            Term::from_field_text(fields.tags, keyword),
            IndexRecordOption::Basic,
        );
        clauses.push((Occur::Should, Box::new(tag_query)));
        let category_query = TermQuery::new(
            Term::from_field_text(fields.category, keyword),
            IndexRecordOption::Basic,
        );
        clauses.push((Occur::Should, Box::new(category_query)));
    }

    if clauses.is_empty() {
        Ok(Some(Box::new(EmptyQuery)))
    } else if clauses.len() == 1 {
        Ok(Some(clauses.remove(0).1))
    } else {
        Ok(Some(Box::new(BooleanQuery::new(clauses))))
    }
}

fn tokenize_terms(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while stream.advance() {
        let token = stream.token();
        if token.text.trim().is_empty() {
            continue;
        }
        tokens.push(token.text.to_string());
    }
    tokens
}

fn build_ngram_tokens_query(
    title: Field,
    subtitle: Field,
    content: Field,
    tokens: &[String],
) -> Option<Box<dyn Query>> {
    if tokens.is_empty() {
        return None;
    }
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for token in tokens {
        let mut token_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let term = Term::from_field_text(title, token);
        token_clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
        ));
        let term = Term::from_field_text(subtitle, token);
        token_clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
        ));
        let term = Term::from_field_text(content, token);
        token_clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
        ));
        clauses.push((Occur::Must, Box::new(BooleanQuery::new(token_clauses))));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn dedup_terms(tokens: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tokens
        .into_iter()
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

fn normalize_cjk(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if is_cjk_char(ch) {
            output.push(ch);
        }
    }
    output
}

fn normalize_latin(input: &str) -> String {
    extract_latin_tokens(input).join(" ")
}

fn extract_latin_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn extract_display_latin_terms(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        if is_display_latin(ch) {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn cjk_bigrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return Vec::new();
    }
    chars
        .windows(2)
        .map(|pair| pair.iter().collect())
        .collect()
}

fn require_field(field: Option<Field>, name: &'static str) -> Result<Field, SearchIndexError> {
    field.ok_or(SearchIndexError::MissingField(name))
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3007}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2F800}'..='\u{2FA1F}'
    )
}

fn build_range_query(field: Field, start: Option<i64>, end: Option<i64>) -> Box<dyn Query> {
    match (start, end) {
        (Some(start), Some(end)) => Box::new(RangeQuery::new(
            Bound::Included(Term::from_field_i64(field, start)),
            Bound::Included(Term::from_field_i64(field, end)),
        )),
        (Some(start), None) => Box::new(RangeQuery::new(
            Bound::Included(Term::from_field_i64(field, start)),
            Bound::Unbounded,
        )),
        (None, Some(end)) => Box::new(RangeQuery::new(
            Bound::Unbounded,
            Bound::Included(Term::from_field_i64(field, end)),
        )),
        (None, None) => Box::new(AllQuery),
    }
}

fn get_string(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)?.as_str().map(|val| val.to_string())
}

fn get_strings(doc: &TantivyDocument, field: Field) -> Vec<String> {
    doc.get_all(field)
        .filter_map(|value| value.as_str().map(|text| text.to_string()))
        .collect()
}

fn get_i64(doc: &TantivyDocument, field: Field) -> Option<i64> {
    doc.get_first(field)?.as_i64()
}

struct HighlightSnippets {
    title: SnippetGenerator,
    subtitle: SnippetGenerator,
    content: SnippetGenerator,
}

fn build_highlight_snippets(
    query: &SearchQuery,
    fields: &SearchFields,
) -> Result<Option<HighlightSnippets>, SearchIndexError> {
    let Some(terms_text) = build_highlight_terms(query) else {
        return Ok(None);
    };
    let analyzer = build_highlight_analyzer();
    let max_chars = 240;
    Ok(Some(HighlightSnippets {
        title: SnippetGenerator::new(
            terms_text.clone(),
            analyzer.clone(),
            fields.title,
            max_chars,
        ),
        subtitle: SnippetGenerator::new(
            terms_text.clone(),
            analyzer.clone(),
            fields.subtitle,
            max_chars,
        ),
        content: SnippetGenerator::new(terms_text, analyzer, fields.content, max_chars),
    }))
}

fn build_highlight_terms(query: &SearchQuery) -> Option<BTreeMap<String, Score>> {
    if query.keywords.is_empty() {
        return None;
    }
    let mut latin_terms = Vec::new();
    let mut cjk_terms = Vec::new();
    for keyword in &query.keywords {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            continue;
        }
        latin_terms.extend(extract_display_latin_terms(keyword));
        let cjk_text = normalize_cjk(keyword);
        let cjk_len = cjk_text.chars().count();
        if cjk_len == 1 {
            cjk_terms.push(cjk_text);
        } else if cjk_len > 1 {
            cjk_terms.extend(cjk_bigrams(&cjk_text));
        }
    }
    let latin_terms = dedup_terms(latin_terms);
    let cjk_terms = dedup_terms(cjk_terms);
    if latin_terms.is_empty() && cjk_terms.is_empty() {
        return None;
    }
    let mut terms_text = BTreeMap::new();
    for term in &latin_terms {
        let normalized = term.to_ascii_lowercase();
        let weight = normalized.chars().count().max(1) as Score;
        terms_text.entry(normalized).or_insert(weight);
    }
    for term in &cjk_terms {
        let weight = term.chars().count().max(1) as Score;
        terms_text.entry(term.clone()).or_insert(weight);
    }
    Some(terms_text)
}

fn build_highlight_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(MixedDisplayTokenizer::default())
        .filter(RemoveLongFilter::limit(80))
        .filter(LowerCaser)
        .build()
}

#[derive(Clone, Default)]
pub struct MixedDisplayTokenizer;

impl Tokenizer for MixedDisplayTokenizer {
    type TokenStream<'a> = MixedDisplayTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        MixedDisplayTokenStream::new(text)
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum RunKind {
    Latin,
    Cjk,
}

fn classify_display_run(ch: char) -> Option<RunKind> {
    if is_cjk_char(ch) {
        Some(RunKind::Cjk)
    } else if is_display_latin(ch) {
        Some(RunKind::Latin)
    } else {
        None
    }
}

fn is_display_latin(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '+' | '#')
}

pub struct MixedDisplayTokenStream<'a> {
    tokens: Vec<Token>,
    i: usize,
    cur: Token,
    _text: &'a str,
}

impl<'a> MixedDisplayTokenStream<'a> {
    pub fn new(text: &'a str) -> Self {
        let tokens = build_display_tokens(text);
        Self {
            tokens,
            i: 0,
            cur: Token::default(),
            _text: text,
        }
    }
}

impl TokenStream for MixedDisplayTokenStream<'_> {
    fn advance(&mut self) -> bool {
        if self.i >= self.tokens.len() {
            return false;
        }
        self.cur = self.tokens[self.i].clone();
        self.i += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.cur
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.cur
    }
}

fn build_display_tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut run_kind: Option<RunKind> = None;
    let mut run_start = 0usize;
    let mut pos = 0usize;

    let flush = |kind: RunKind,
                 start: usize,
                 end: usize,
                 out: &mut Vec<Token>,
                 pos: &mut usize| {
        if start >= end {
            return;
        }
        let slice = &text[start..end];
        match kind {
            RunKind::Latin => {
                let mut tok = Token::default();
                tok.offset_from = start;
                tok.offset_to = end;
                tok.position = *pos;
                tok.position_length = 1;
                tok.text = slice.to_string();
                out.push(tok);
                *pos += 1;
            }
            RunKind::Cjk => {
                let mut idx: Vec<usize> = slice.char_indices().map(|(i, _)| i).collect();
                idx.push(slice.len());
                let n = idx.len().saturating_sub(1);
                for i in 0..n {
                    let a = start + idx[i];
                    let b = start + idx[i + 1];
                    let mut tok = Token::default();
                    tok.offset_from = a;
                    tok.offset_to = b;
                    tok.position = *pos;
                    tok.position_length = 1;
                    tok.text = text[a..b].to_string();
                    out.push(tok);
                    *pos += 1;
                }
                for i in 0..n.saturating_sub(1) {
                    let a = start + idx[i];
                    let b = start + idx[i + 2];
                    let mut tok = Token::default();
                    tok.offset_from = a;
                    tok.offset_to = b;
                    tok.position = *pos;
                    tok.position_length = 1;
                    tok.text = text[a..b].to_string();
                    out.push(tok);
                    *pos += 1;
                }
            }
        }
    };

    for (i, ch) in text.char_indices() {
        let kind = classify_display_run(ch);
        match (run_kind, kind) {
            (None, Some(next)) => {
                run_kind = Some(next);
                run_start = i;
            }
            (Some(current), Some(next)) if current == next => {}
            (Some(current), Some(next)) => {
                flush(current, run_start, i, &mut out, &mut pos);
                run_kind = Some(next);
                run_start = i;
            }
            (Some(current), None) => {
                flush(current, run_start, i, &mut out, &mut pos);
                run_kind = None;
            }
            (None, None) => {}
        }
    }
    if let Some(current) = run_kind {
        flush(current, run_start, text.len(), &mut out, &mut pos);
    }

    out.sort_by(|left, right| {
        (left.offset_from, left.offset_to).cmp(&(right.offset_from, right.offset_to))
    });
    for (idx, token) in out.iter_mut().enumerate() {
        token.position = idx;
        token.position_length = 1;
    }
    out
}

fn snippet_html(generator: Option<&SnippetGenerator>, doc: &TantivyDocument) -> Option<String> {
    let generator = generator?;
    let snippet = generator.snippet_from_doc(doc);
    let html = snippet.to_html();
    let trimmed = html.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn snippet_or_excerpt(
    generator: Option<&SnippetGenerator>,
    doc: &TantivyDocument,
    field: Field,
    max_chars: usize,
) -> Option<String> {
    if let Some(snippet) = snippet_html(generator, doc) {
        return Some(snippet);
    }
    let text = get_string(doc, field)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(max_chars).collect())
    }
}

fn timestamp_to_datetime(ts: i64, field: &'static str) -> Result<DateTime<Utc>, SearchIndexError> {
    DateTime::<Utc>::from_timestamp(ts, 0).ok_or(SearchIndexError::InvalidTimestamp(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::collector::TopDocs;
    use tantivy::doc;

    fn add_ngram_fields(
        doc: &mut TantivyDocument,
        fields: &SearchFields,
        title: &str,
        subtitle: Option<&str>,
        content: &str,
    ) {
        let title_cjk = normalize_cjk(title);
        if let Some(field) = fields.title_cjk_bg {
            doc.add_text(field, &title_cjk);
        }
        if let Some(field) = fields.title_cjk_ug {
            doc.add_text(field, &title_cjk);
        }
        let subtitle_cjk = subtitle.map(normalize_cjk).unwrap_or_default();
        if let Some(field) = fields.subtitle_cjk_bg {
            doc.add_text(field, &subtitle_cjk);
        }
        if let Some(field) = fields.subtitle_cjk_ug {
            doc.add_text(field, &subtitle_cjk);
        }
        let content_cjk = normalize_cjk(content);
        if let Some(field) = fields.content_cjk_bg {
            doc.add_text(field, &content_cjk);
        }
        if let Some(field) = fields.content_cjk_ug {
            doc.add_text(field, &content_cjk);
        }
        let title_latin = normalize_latin(title);
        if let Some(field) = fields.title_latin {
            doc.add_text(field, &title_latin);
        }
        let subtitle_latin = subtitle.map(normalize_latin).unwrap_or_default();
        if let Some(field) = fields.subtitle_latin {
            doc.add_text(field, &subtitle_latin);
        }
        let content_latin = normalize_latin(content);
        if let Some(field) = fields.content_latin {
            doc.add_text(field, &content_latin);
        }
    }

    fn build_index() -> Result<(Index, SearchFields), SearchIndexError> {
        let schema = build_schema();
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index)?;
        let fields = SearchFields::from_schema(&index.schema())?;
        Ok((index, fields))
    }

    #[test]
    fn ngram_search_finds_cjk_and_highlight() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let subtitle = fields.subtitle;
        let content = fields.content;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "张华考上了北京大学；我在百货公司当售货员",
            subtitle => "副标题无关内容",
            content => "百货公司里有一个售货员正在忙碌。"
        );
        add_ngram_fields(
            &mut doc,
            &fields,
            "张华考上了北京大学；我在百货公司当售货员",
            Some("副标题无关内容"),
            "百货公司里有一个售货员正在忙碌。",
        );
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["售货员".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        let doc_address = top_docs[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;

        let highlight = build_highlight_snippets(&search_query, &fields)?.expect("highlight");
        let title_html = snippet_html(Some(&highlight.title), &doc).unwrap_or_default();
        let content_html = snippet_html(Some(&highlight.content), &doc).unwrap_or_default();
        assert!(title_html.contains("售货员"));
        assert!(content_html.contains("售货员"));
        Ok(())
    }

    #[test]
    fn keyword_query_matches_subtitle() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let subtitle = fields.subtitle;
        let content = fields.content;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "无关标题",
            subtitle => "这里有关键词",
            content => "无关正文"
        );
        add_ngram_fields(&mut doc, &fields, "无关标题", Some("这里有关键词"), "无关正文");
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["关键词".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        let doc_address = top_docs[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let subtitle_value = get_string(&doc, subtitle).unwrap_or_default();
        assert!(subtitle_value.contains("关键词"));
        Ok(())
    }

    #[test]
    fn title_snippet_falls_back_to_title_text() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let subtitle = fields.subtitle;
        let content = fields.content;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "离职，三年未满",
            subtitle => "副标题",
            content => "正文"
        );
        add_ngram_fields(&mut doc, &fields, "离职，三年未满", Some("副标题"), "正文");
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["正文".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(1))?;
        let doc_address = top_docs[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;

        let highlight = build_highlight_snippets(&search_query, &fields)?.expect("highlight");
        let snippet = snippet_or_excerpt(Some(&highlight.title), &doc, title, 120).unwrap();
        assert!(snippet.contains("离职"));
        Ok(())
    }

    #[test]
    fn ngram_search_splits_cjk_and_latin_keywords() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let subtitle = fields.subtitle;
        let content = fields.content;
        let text = "我的 podman 自建全部服务方案";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => text,
            subtitle => "",
            content => text
        );
        add_ngram_fields(&mut doc, &fields, text, Some(""), text);
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec![
                "我的".to_string(),
                "podman".to_string(),
                "自建全部服务方案".to_string(),
            ],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!hits.is_empty());
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let title_value = get_string(&doc, title).unwrap_or_default();
        let content_value = get_string(&doc, content).unwrap_or_default();
        let subtitle_value = get_string(&doc, subtitle).unwrap_or_default();
        assert!(title_value.contains("podman") || content_value.contains("podman") || subtitle_value.contains("podman"));
        Ok(())
    }

    #[test]
    fn ngram_search_requires_all_keywords() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let text = "昨天";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => text,
            content => text
        );
        add_ngram_fields(&mut doc, &fields, text, None, text);
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["昨天".to_string(), "今天".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(hits.is_empty());
        Ok(())
    }

    #[test]
    fn ngram_search_allows_single_cjk_as_optional_with_other_keywords(
    ) -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let text = "昨天";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => text,
            content => text
        );
        add_ngram_fields(&mut doc, &fields, text, None, text);
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["昨天".to_string(), "爱".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!hits.is_empty());
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let title_value = get_string(&doc, title).unwrap_or_default();
        let content_value = get_string(&doc, content).unwrap_or_default();
        assert!(title_value.contains("昨天") || content_value.contains("昨天"));
        Ok(())
    }

    #[test]
    fn ngram_search_uses_unigram_for_single_cjk() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let text = "我爱Rust";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => text,
            content => text
        );
        add_ngram_fields(&mut doc, &fields, text, None, text);
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["爱".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!hits.is_empty());
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let title_value = get_string(&doc, title).unwrap_or_default();
        let content_value = get_string(&doc, content).unwrap_or_default();
        assert!(title_value.contains("爱") || content_value.contains("爱"));
        Ok(())
    }

    #[test]
    fn ngram_search_requires_all_tokens_for_keyword() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let text = "数据系统分析";
        let mut doc = doc!(
            title => text,
            content => text
        );
        add_ngram_fields(&mut doc, &fields, text, None, text);
        writer.add_document(doc)?;
        let matched = "数据分析";
        let mut doc = doc!(
            title => matched,
            content => matched
        );
        add_ngram_fields(&mut doc, &fields, matched, None, matched);
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["数据分析".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert_eq!(hits.len(), 1);
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let title_value = get_string(&doc, title).unwrap_or_default();
        assert_eq!(title_value, matched);
        Ok(())
    }

    #[test]
    fn highlight_snippet_uses_display_terms() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let fields = SearchFields::from_schema(&schema)?;
        let doc: TantivyDocument = doc!(
            title => "无关标题",
            subtitle => "副标题",
            content => "终究还是走上了自建的路。这是一个 suspense 故事，使用 resvg-js 渲染。"
        );
        let search_query = SearchQuery {
            keywords: vec![
                "自建".to_string(),
                "suspense".to_string(),
                "resvg-js".to_string(),
            ],
            ..Default::default()
        };
        let highlight = build_highlight_snippets(&search_query, &fields)?.expect("highlight");
        let content_html = snippet_html(Some(&highlight.content), &doc).unwrap_or_default();
        assert!(content_html.contains("<b>自建</b>"));
        assert!(content_html.to_ascii_lowercase().contains("<b>suspense</b>"));
        assert!(content_html.to_ascii_lowercase().contains("<b>resvg-js</b>"));
        let title_html = snippet_html(Some(&highlight.title), &doc).unwrap_or_default();
        assert!(!title_html.contains("<b>"));
        Ok(())
    }

    #[test]
    fn mixed_display_token_offsets_are_monotonic() {
        let text = "数据分析 suspense resvg-js";
        let mut tokenizer = MixedDisplayTokenizer::default();
        let mut stream = tokenizer.token_stream(text);
        let mut last_from = 0usize;
        let mut last_to = 0usize;
        let mut first = true;
        while stream.advance() {
            let token = stream.token();
            assert!(token.offset_from <= token.offset_to);
            if !first {
                assert!(token.offset_from >= last_from);
                assert!(token.offset_to >= last_to);
            }
            last_from = token.offset_from;
            last_to = token.offset_to;
            first = false;
        }
    }

    #[test]
    fn tag_filter_matches_tags() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let tags = fields.tags;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "无关标题",
            content => "无关正文",
            tags => "售货员"
        );
        add_ngram_fields(&mut doc, &fields, "无关标题", None, "无关正文");
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            tags: vec!["售货员".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let tags_value = get_strings(&doc, tags);
        assert!(tags_value.iter().any(|tag| tag == "售货员"));
        Ok(())
    }

    #[test]
    fn range_query_matches_updated() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let published = fields.published;
        let updated = fields.updated;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "只更新命中",
            content => "正文内容",
            published => 1_577_836_800i64,
            updated => 1_735_689_600i64
        );
        add_ngram_fields(&mut doc, &fields, "只更新命中", None, "正文内容");
        writer.add_document(doc)?;
        writer.commit()?;

        let range = inkstone_core::types::time_range::TimeRange::parse("2024-01-01~2026-01-01")
            .unwrap();
        let search_query = SearchQuery {
            range: Some(range),
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let searcher = index.reader_builder().try_into()?.searcher();
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }

    #[test]
    fn category_filter_matches_category() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let category = fields.category;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "无关标题",
            content => "无关正文",
            category => "实验室"
        );
        add_ngram_fields(&mut doc, &fields, "无关标题", None, "无关正文");
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            category: Some("实验室".to_string()),
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let category_value = get_string(&doc, category).unwrap_or_default();
        assert_eq!(category_value, "实验室");
        Ok(())
    }

    #[test]
    fn keyword_query_requires_all_keywords() -> Result<(), SearchIndexError> {
        let (index, fields) = build_index()?;
        let title = fields.title;
        let content = fields.content;
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let mut doc = doc!(
            title => "Rust 语言",
            content => "只有 Rust"
        );
        add_ngram_fields(&mut doc, &fields, "Rust 语言", None, "只有 Rust");
        writer.add_document(doc)?;
        let mut doc = doc!(
            title => "Rust 搜索",
            content => "Rust 搜索 都有"
        );
        add_ngram_fields(&mut doc, &fields, "Rust 搜索", None, "Rust 搜索 都有");
        writer.add_document(doc)?;
        writer.commit()?;

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["Rust".to_string(), "搜索".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let hit_title = get_string(&doc, title).unwrap_or_default();
        let hit_content = get_string(&doc, content).unwrap_or_default();
        assert!(hit_title.contains("搜索") || hit_content.contains("搜索"));
        Ok(())
    }
}
