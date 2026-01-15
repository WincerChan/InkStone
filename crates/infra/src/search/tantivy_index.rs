use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use inkstone_core::domain::search::{SearchDocument, SearchHit, SearchQuery, SearchResult};
use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use std::ops::Bound;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{
    AllQuery, BooleanQuery, EmptyQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery,
};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, SchemaBuilder, TextFieldIndexing, TextOptions, Value, FAST,
    STORED, STRING,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{
    LowerCaser, NgramTokenizer, RemoveLongFilter, SimpleTokenizer, Stemmer, TextAnalyzer,
};
use tantivy::{DocAddress, Index, IndexReader, Order, ReloadPolicy, TantivyDocument, Term};
use thiserror::Error;

use super::{SearchSort, SearchStrategy};

const TOKENIZER_JIEBA: &str = "jieba";
const TOKENIZER_LINDERA: &str = "lindera";
const TOKENIZER_NGRAM: &str = "ngram";
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
    #[error("lindera error: {0}")]
    Lindera(String),
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
    strategy: SearchStrategy,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexStats {
    pub num_docs: u64,
    pub num_segments: usize,
}

impl SearchIndex {
    pub fn open_or_create(
        path: impl AsRef<Path>,
        strategy: SearchStrategy,
    ) -> Result<Self, SearchIndexError> {
        let dir = path.as_ref();
        std::fs::create_dir_all(dir)?;

        let schema = build_schema(strategy);
        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir)?
        } else {
            Index::create_in_dir(dir, schema)?
        };
        register_tokenizers(&index, strategy)?;
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
            strategy,
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
        let built_query = build_query(&self.index, &self.fields, query, self.strategy)?;
        let (title_snippet, subtitle_snippet, content_snippet) =
            if let Some(keyword_query) = built_query.keyword.as_ref() {
                let mut title_snippet =
                    SnippetGenerator::create(&searcher, &**keyword_query, self.fields.title)?;
                title_snippet.set_max_num_chars(240);
                let mut subtitle_snippet =
                    SnippetGenerator::create(&searcher, &**keyword_query, self.fields.subtitle)?;
                subtitle_snippet.set_max_num_chars(240);
                let mut content_snippet =
                    SnippetGenerator::create(&searcher, &**keyword_query, self.fields.content)?;
                content_snippet.set_max_num_chars(240);
                (Some(title_snippet), Some(subtitle_snippet), Some(content_snippet))
            } else {
                (None, None, None)
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
                title_snippet.as_ref(),
                subtitle_snippet.as_ref(),
                content_snippet.as_ref(),
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
        if self.strategy == SearchStrategy::Ngram {
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
    use super::{SearchIndex, SearchStrategy};
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
        let index = SearchIndex::open_or_create(&dir, SearchStrategy::Jieba).unwrap();
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

fn build_schema(strategy: SearchStrategy) -> Schema {
    let mut builder = SchemaBuilder::default();
    builder.add_text_field("id", STRING | STORED);
    let (title_opts, subtitle_opts, content_opts) = match strategy {
        SearchStrategy::Jieba => (
            jieba_text_options(true),
            jieba_text_options(true),
            jieba_text_options(true),
        ),
        SearchStrategy::Lindera => (
            lindera_text_options(true),
            lindera_text_options(true),
            lindera_text_options(true),
        ),
        SearchStrategy::Ngram => (
            ngram_text_options(true, true),
            ngram_text_options(true, true),
            ngram_text_options(true, true),
        ),
    };
    builder.add_text_field("title", title_opts);
    builder.add_text_field("subtitle", subtitle_opts);
    builder.add_text_field("content", content_opts);
    if strategy == SearchStrategy::Ngram {
        builder.add_text_field("title_cjk_bg", cjk_bigram_text_options(false, false));
        builder.add_text_field("subtitle_cjk_bg", cjk_bigram_text_options(false, false));
        builder.add_text_field("content_cjk_bg", cjk_bigram_text_options(false, false));
        builder.add_text_field("title_cjk_ug", cjk_unigram_text_options(false, false));
        builder.add_text_field("subtitle_cjk_ug", cjk_unigram_text_options(false, false));
        builder.add_text_field("content_cjk_ug", cjk_unigram_text_options(false, false));
        builder.add_text_field("title_latin", latin_text_options(false, false));
        builder.add_text_field("subtitle_latin", latin_text_options(false, false));
        builder.add_text_field("content_latin", latin_text_options(false, false));
    }
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("tags", STRING | STORED);
    builder.add_text_field("category", STRING | STORED);
    builder.add_i64_field("published", STORED | FAST);
    builder.add_i64_field("updated", STORED | FAST);
    builder.add_text_field("checksum", STRING | STORED);
    builder.build()
}

fn jieba_text_options(stored: bool) -> TextOptions {
    text_options(TOKENIZER_JIEBA, stored, IndexRecordOption::WithFreqsAndPositions)
}

fn lindera_text_options(stored: bool) -> TextOptions {
    text_options(
        TOKENIZER_LINDERA,
        stored,
        IndexRecordOption::WithFreqsAndPositions,
    )
}

fn ngram_text_options(stored: bool, with_positions: bool) -> TextOptions {
    let option = if with_positions {
        IndexRecordOption::WithFreqsAndPositions
    } else {
        IndexRecordOption::WithFreqs
    };
    text_options(TOKENIZER_NGRAM, stored, option)
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

fn register_tokenizers(index: &Index, strategy: SearchStrategy) -> Result<(), SearchIndexError> {
    match strategy {
        SearchStrategy::Jieba => {
            let analyzer = build_jieba_analyzer();
            index.tokenizers().register(TOKENIZER_JIEBA, analyzer);
            Ok(())
        }
        SearchStrategy::Lindera => {
            let analyzer = build_lindera_analyzer()?;
            index.tokenizers().register(TOKENIZER_LINDERA, analyzer);
            Ok(())
        }
        SearchStrategy::Ngram => {
            let analyzer = build_ngram_analyzer()?;
            index.tokenizers().register(TOKENIZER_NGRAM, analyzer);
            let analyzer = build_cjk_bigram_analyzer()?;
            index.tokenizers().register(TOKENIZER_CJK_BG, analyzer);
            let analyzer = build_cjk_unigram_analyzer()?;
            index.tokenizers().register(TOKENIZER_CJK_UG, analyzer);
            let analyzer = build_latin_analyzer();
            index.tokenizers().register(TOKENIZER_LATIN, analyzer);
            Ok(())
        }
    }
}

fn build_jieba_analyzer() -> TextAnalyzer {
    let tokenizer = tantivy_jieba::JiebaTokenizer {};
    TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build()
}

fn build_lindera_analyzer() -> Result<TextAnalyzer, SearchIndexError> {
    let dictionary = load_dictionary("embedded://cc-cedict")
        .map_err(|err| SearchIndexError::Lindera(err.to_string()))?;
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    Ok(TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build())
}

fn build_ngram_analyzer() -> Result<TextAnalyzer, SearchIndexError> {
    build_cjk_ngram_analyzer(1, 2)
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
    strategy: SearchStrategy,
) -> Result<BuiltQuery, SearchIndexError> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    let search_keyword_query =
        build_keyword_query(index, fields, query, strategy, KeywordQueryKind::Search)?;
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
    let snippet_query =
        build_keyword_query(index, fields, query, strategy, KeywordQueryKind::Snippet)?;
    Ok(BuiltQuery {
        query: compiled_query,
        keyword: snippet_query,
    })
}

struct BuiltQuery {
    query: Box<dyn Query>,
    keyword: Option<Box<dyn Query>>,
}

#[derive(Clone, Copy)]
enum KeywordQueryKind {
    Search,
    Snippet,
}

fn build_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
    strategy: SearchStrategy,
    kind: KeywordQueryKind,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    if query.keywords.is_empty() {
        return Ok(None);
    }
    match strategy {
        SearchStrategy::Jieba => build_dictionary_keyword_query(
            index,
            fields,
            query,
            kind,
            TOKENIZER_JIEBA,
        ),
        SearchStrategy::Lindera => build_dictionary_keyword_query(
            index,
            fields,
            query,
            kind,
            TOKENIZER_LINDERA,
        ),
        SearchStrategy::Ngram => build_ngram_keyword_query(index, fields, query, kind),
    }
}

fn build_dictionary_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
    _kind: KeywordQueryKind,
    tokenizer: &'static str,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    let mut analyzer = index
        .tokenizers()
        .get(tokenizer)
        .ok_or(SearchIndexError::MissingTokenizer(tokenizer))?;
    let mut keyword_queries = Vec::new();
    for keyword in &query.keywords {
        let tokens = tokenize_keyword(&mut analyzer, keyword);
        let title_query = build_field_query(fields.title, &tokens);
        let subtitle_query = build_field_query(fields.subtitle, &tokens);
        let content_query = build_field_query(fields.content, &tokens);
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        if let Some(query) = title_query {
            clauses.push((Occur::Should, query));
        }
        if let Some(query) = subtitle_query {
            clauses.push((Occur::Should, query));
        }
        if let Some(query) = content_query {
            clauses.push((Occur::Should, query));
        }
        if !keyword.is_empty() {
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
        let keyword_query = if clauses.is_empty() {
            None
        } else {
            Some(Box::new(BooleanQuery::new(clauses)) as Box<dyn Query>)
        };
        if let Some(keyword_query) = keyword_query {
            keyword_queries.push(keyword_query);
        }
    }

    if keyword_queries.is_empty() {
        return Ok(Some(Box::new(EmptyQuery)));
    }
    if keyword_queries.len() == 1 {
        Ok(Some(keyword_queries.remove(0)))
    } else {
        Ok(Some(Box::new(BooleanQuery::new(
            keyword_queries
                .into_iter()
                .map(|query| (Occur::Must, query))
                .collect(),
        ))))
    }
}

fn build_ngram_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
    kind: KeywordQueryKind,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    let keyword_text = query.keywords.join(" ");
    let trimmed = keyword_text.trim();
    if trimmed.is_empty() {
        return Ok(Some(Box::new(EmptyQuery)));
    }

    match kind {
        KeywordQueryKind::Search => {
            let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for keyword in &query.keywords {
                let keyword = keyword.trim();
                if keyword.is_empty() {
                    continue;
                }
                let mut keyword_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

                let cjk_text = normalize_cjk(keyword);
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
                        if let Some(query) =
                            build_ngram_tokens_query(title, subtitle, content, &tokens)
                        {
                            keyword_clauses.push((Occur::Must, query));
                        }
                    }
                }

                let latin_raw = extract_latin_tokens(keyword);
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
                        if let Some(query) =
                            build_ngram_tokens_query(title, subtitle, content, &tokens)
                        {
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
                    clauses.push((Occur::Must, keyword_query));
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
        KeywordQueryKind::Snippet => {
            let mut analyzer = index
                .tokenizers()
                .get(TOKENIZER_NGRAM)
                .ok_or(SearchIndexError::MissingTokenizer(TOKENIZER_NGRAM))?;
            let tokens = dedup_terms(tokenize_terms(&mut analyzer, trimmed));
            if tokens.is_empty() {
                return Ok(Some(Box::new(EmptyQuery)));
            }
            let Some(query) =
                build_ngram_fields_query(fields.title, fields.subtitle, fields.content, &tokens)
            else {
                return Ok(Some(Box::new(EmptyQuery)));
            };
            Ok(Some(query))
        }
    }
}

fn tokenize_keyword(
    analyzer: &mut TextAnalyzer,
    keyword: &str,
) -> Vec<(usize, String)> {
    let mut stream = analyzer.token_stream(keyword);
    let mut tokens = Vec::new();
    while stream.advance() {
        let token = stream.token();
        if token.text.trim().is_empty() {
            continue;
        }
        tokens.push((token.position, token.text.to_string()));
    }
    tokens
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

fn build_ngram_fields_query(
    title: Field,
    subtitle: Field,
    content: Field,
    tokens: &[String],
) -> Option<Box<dyn Query>> {
    if tokens.is_empty() {
        return None;
    }
    let title_query = build_ngram_field_query(title, tokens);
    let subtitle_query = build_ngram_field_query(subtitle, tokens);
    let content_query = build_ngram_field_query(content, tokens);
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    if let Some(query) = title_query {
        clauses.push((Occur::Should, query));
    }
    if let Some(query) = subtitle_query {
        clauses.push((Occur::Should, query));
    }
    if let Some(query) = content_query {
        clauses.push((Occur::Should, query));
    }
    if clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(clauses)))
    }
}

fn build_ngram_field_query(field: Field, tokens: &[String]) -> Option<Box<dyn Query>> {
    if tokens.is_empty() {
        return None;
    }
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for token in tokens {
        let term = Term::from_field_text(field, token);
        clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)),
        ));
    }
    Some(Box::new(BooleanQuery::new(clauses)))
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

fn build_field_query(field: Field, tokens: &[(usize, String)]) -> Option<Box<dyn Query>> {
    match tokens.len() {
        0 => None,
        1 => Some(Box::new(TermQuery::new(
            Term::from_field_text(field, &tokens[0].1),
            IndexRecordOption::WithFreqs,
        ))),
        _ => {
            let terms = tokens
                .iter()
                .map(|(pos, text)| (*pos, Term::from_field_text(field, text)))
                .collect();
            Some(Box::new(PhraseQuery::new_with_offset(terms)))
        }
    }
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
    use serde::Deserialize;
    use tantivy::collector::TopDocs;
    use tantivy::doc;

    #[test]
    fn jieba_tokenizer_searches_chinese() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "张华考上了北京大学；我在百货公司当售货员",
            subtitle => "副标题无关内容",
            content => "百货公司里有一个售货员正在忙碌。"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["售货员".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        let doc_address = top_docs[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;

        let keyword_query = built.keyword.as_ref().expect("keyword query");
        let mut title_snippet = SnippetGenerator::create(&searcher, &**keyword_query, title)?;
        title_snippet.set_max_num_chars(240);
        let mut content_snippet = SnippetGenerator::create(&searcher, &**keyword_query, content)?;
        content_snippet.set_max_num_chars(240);
        let title_html = title_snippet.snippet_from_doc(&doc).to_html();
        let content_html = content_snippet.snippet_from_doc(&doc).to_html();
        assert!(title_html.contains("售货员"));
        assert!(content_html.contains("售货员"));
        Ok(())
    }

    #[test]
    fn jieba_tokenizer_outputs_tokens_for_content() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut analyzer = index
            .tokenizers()
            .get("jieba")
            .expect("jieba tokenizer");
        let content = "临近我 28 岁生日时，我原本并没有写文章的打算。";
        let mut stream = analyzer.token_stream(content);
        let mut tokens = Vec::new();
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        println!("jieba tokens: {:?}", tokens);
        assert!(!tokens.is_empty());
        Ok(())
    }

    #[test]
    fn jieba_vs_lindera_tokenization_on_corpus() -> Result<(), SearchIndexError> {
        let mut jieba_analyzer = build_jieba_analyzer();
        let mut lindera_analyzer = build_lindera_analyzer()?;
        let samples = load_corpus_samples(100)?;

        let mut total = 0usize;
        let mut same = 0usize;
        for sample in samples {
            let jieba_tokens = tokenize_with_analyzer(&mut jieba_analyzer, &sample);
            let lindera_tokens = tokenize_with_analyzer(&mut lindera_analyzer, &sample);
            total += 1;
            if jieba_tokens == lindera_tokens {
                same += 1;
            }
        }
        println!(
            "lindera vs jieba tokenization: total={total} same={same} diff={diff}",
            diff = total.saturating_sub(same)
        );

        let jieba_keyword = tokenize_with_analyzer(&mut jieba_analyzer, "自建");
        let lindera_keyword = tokenize_with_analyzer(&mut lindera_analyzer, "自建");
        println!("jieba keyword tokens: {:?}", jieba_keyword);
        println!("lindera keyword tokens: {:?}", lindera_keyword);
        Ok(())
    }

    #[test]
    fn jieba_searches_three_years_phrase_in_title() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "离职，三年未满",
            subtitle => "副标题",
            content => "正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["三年".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        Ok(())
    }

    #[test]
    fn keyword_query_matches_subtitle() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            subtitle => "这里有关键词",
            content => "无关正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["关键词".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        Ok(())
    }

    #[test]
    fn keyword_query_matches_chinese_subtitle_token() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            subtitle => "终究还是走上了自建的路",
            content => "无关正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["自建".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        Ok(())
    }

    #[test]
    fn title_snippet_falls_back_to_title_text() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "离职，三年未满",
            subtitle => "副标题",
            content => "正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["正文".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(1))?;
        let doc_address = top_docs[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;

        let keyword_query = built.keyword.as_ref().expect("keyword query");
        let mut title_snippet = SnippetGenerator::create(&searcher, &**keyword_query, title)?;
        title_snippet.set_max_num_chars(240);
        let snippet = snippet_or_excerpt(Some(&title_snippet), &doc, title, 120).unwrap();
        assert!(snippet.contains("离职"));
        Ok(())
    }

    #[test]
    fn jieba_tokenizer_outputs_tokens_for_jian_examples() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut analyzer = index
            .tokenizers()
            .get("jieba")
            .expect("jieba tokenizer");

        let keyword = "自建";
        let content = "终究还是走上了自建的路";
        let keyword_tokens: Vec<String> = {
            let mut stream = analyzer.token_stream(keyword);
            let mut tokens = Vec::new();
            while stream.advance() {
                tokens.push(stream.token().text.to_string());
            }
            tokens
        };
        let content_tokens: Vec<String> = {
            let mut stream = analyzer.token_stream(content);
            let mut tokens = Vec::new();
            while stream.advance() {
                tokens.push(stream.token().text.to_string());
            }
            tokens
        };

        println!("jieba keyword tokens: {:?}", keyword_tokens);
        println!("jieba content tokens: {:?}", content_tokens);
        Ok(())
    }

    #[test]
    fn tokenizer_outputs_tokens_for_suspense() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Ngram);
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;
        register_tokenizers(&index, SearchStrategy::Lindera)?;
        register_tokenizers(&index, SearchStrategy::Ngram)?;

        let input = "suspense";
        let mut jieba_analyzer = index
            .tokenizers()
            .get(TOKENIZER_JIEBA)
            .expect("jieba tokenizer");
        let mut lindera_analyzer = index
            .tokenizers()
            .get(TOKENIZER_LINDERA)
            .expect("lindera tokenizer");
        let mut latin_analyzer = index
            .tokenizers()
            .get(TOKENIZER_LATIN)
            .expect("latin tokenizer");

        let jieba_tokens = tokenize_terms(&mut jieba_analyzer, input);
        let lindera_tokens = tokenize_terms(&mut lindera_analyzer, input);
        let latin_tokens = tokenize_terms(&mut latin_analyzer, input);

        println!("jieba tokens (suspense): {:?}", jieba_tokens);
        println!("lindera tokens (suspense): {:?}", lindera_tokens);
        println!("latin tokens (suspense): {:?}", latin_tokens);
        Ok(())
    }

    #[test]
    fn ngram_search_splits_cjk_and_latin_keywords() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Ngram);
        let title = schema.get_field("title")?;
        let subtitle = schema.get_field("subtitle")?;
        let content = schema.get_field("content")?;
        let title_cjk_bg = schema.get_field("title_cjk_bg")?;
        let content_cjk_bg = schema.get_field("content_cjk_bg")?;
        let title_cjk_ug = schema.get_field("title_cjk_ug")?;
        let content_cjk_ug = schema.get_field("content_cjk_ug")?;
        let title_latin = schema.get_field("title_latin")?;
        let content_latin = schema.get_field("content_latin")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Ngram)?;

        let text = "我的 podman 自建全部服务方案";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => text,
            subtitle => "",
            content => text,
            title_cjk_bg => normalize_cjk(text),
            content_cjk_bg => normalize_cjk(text),
            title_cjk_ug => normalize_cjk(text),
            content_cjk_ug => normalize_cjk(text),
            title_latin => normalize_latin(text),
            content_latin => normalize_latin(text)
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec![
                "我的".to_string(),
                "podman".to_string(),
                "自建全部服务方案".to_string(),
            ],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Ngram)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!hits.is_empty());
        Ok(())
    }

    #[test]
    fn ngram_search_requires_all_keywords() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Ngram);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let title_cjk_bg = schema.get_field("title_cjk_bg")?;
        let content_cjk_bg = schema.get_field("content_cjk_bg")?;
        let title_cjk_ug = schema.get_field("title_cjk_ug")?;
        let content_cjk_ug = schema.get_field("content_cjk_ug")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Ngram)?;

        let text = "昨天";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => text,
            content => text,
            title_cjk_bg => normalize_cjk(text),
            content_cjk_bg => normalize_cjk(text),
            title_cjk_ug => normalize_cjk(text),
            content_cjk_ug => normalize_cjk(text)
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["昨天".to_string(), "爱".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Ngram)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(hits.is_empty());
        Ok(())
    }

    #[test]
    fn ngram_search_uses_unigram_for_single_cjk() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Ngram);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let title_cjk_bg = schema.get_field("title_cjk_bg")?;
        let content_cjk_bg = schema.get_field("content_cjk_bg")?;
        let title_cjk_ug = schema.get_field("title_cjk_ug")?;
        let content_cjk_ug = schema.get_field("content_cjk_ug")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Ngram)?;

        let text = "我爱Rust";
        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => text,
            content => text,
            title_cjk_bg => normalize_cjk(text),
            content_cjk_bg => normalize_cjk(text),
            title_cjk_ug => normalize_cjk(text),
            content_cjk_ug => normalize_cjk(text)
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["爱".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Ngram)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!hits.is_empty());
        Ok(())
    }

    #[test]
    fn ngram_search_requires_all_tokens_for_keyword() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Ngram);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let title_cjk_bg = schema.get_field("title_cjk_bg")?;
        let content_cjk_bg = schema.get_field("content_cjk_bg")?;
        let title_cjk_ug = schema.get_field("title_cjk_ug")?;
        let content_cjk_ug = schema.get_field("content_cjk_ug")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Ngram)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        let text = "数据系统分析";
        writer.add_document(doc!(
            title => text,
            content => text,
            title_cjk_bg => normalize_cjk(text),
            content_cjk_bg => normalize_cjk(text),
            title_cjk_ug => normalize_cjk(text),
            content_cjk_ug => normalize_cjk(text)
        ))?;
        let matched = "数据分析";
        writer.add_document(doc!(
            title => matched,
            content => matched,
            title_cjk_bg => normalize_cjk(matched),
            content_cjk_bg => normalize_cjk(matched),
            title_cjk_ug => normalize_cjk(matched),
            content_cjk_ug => normalize_cjk(matched)
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["数据分析".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Ngram)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert_eq!(hits.len(), 1);
        let doc_address = hits[0].1;
        let doc: TantivyDocument = searcher.doc(doc_address)?;
        let title_value = get_string(&doc, title).unwrap_or_default();
        assert_eq!(title_value, matched);
        Ok(())
    }

    #[derive(Debug, Deserialize)]
    struct SearchIndexEntry {
        title: String,
        subtitle: Option<String>,
        content: String,
    }

    fn load_corpus_samples(min_samples: usize) -> Result<Vec<String>, SearchIndexError> {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map_err(|err| SearchIndexError::Io(std::io::Error::new(std::io::ErrorKind::Other, err)))?;
        let path = Path::new(&manifest_dir)
            .join("..")
            .join("..")
            .join("search-index.json");
        let content = std::fs::read_to_string(path)?;
        let entries: Vec<SearchIndexEntry> =
            serde_json::from_str(&content).map_err(|err| {
                SearchIndexError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, err))
            })?;
        let mut samples = Vec::new();
        for entry in entries {
            if !entry.title.trim().is_empty() {
                samples.push(entry.title);
            }
            if let Some(subtitle) = entry.subtitle {
                if !subtitle.trim().is_empty() {
                    samples.push(subtitle);
                }
            }
            if !entry.content.trim().is_empty() {
                let snippet = entry.content.chars().take(120).collect::<String>();
                samples.push(snippet);
            }
        }
        if samples.len() < min_samples {
            return Err(SearchIndexError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("corpus samples too small: {}", samples.len()),
            )));
        }
        Ok(samples)
    }

    fn tokenize_with_analyzer(analyzer: &mut TextAnalyzer, text: &str) -> Vec<String> {
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

    #[test]
    fn keyword_query_matches_tags() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let tags = schema.get_field("tags")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            content => "无关正文",
            tags => "售货员"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["售货员".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }

    #[test]
    fn range_query_matches_updated() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let published = schema.get_field("published")?;
        let updated = schema.get_field("updated")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "只更新命中",
            content => "正文内容",
            published => 1_577_836_800i64,
            updated => 1_735_689_600i64
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;

        let range = inkstone_core::types::time_range::TimeRange::parse("2024-01-01~2026-01-01")
            .unwrap();
        let search_query = SearchQuery {
            range: Some(range),
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let searcher = index.reader_builder().try_into()?.searcher();
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }

    #[test]
    fn keyword_query_matches_category() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let category = schema.get_field("category")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            content => "无关正文",
            category => "实验室"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["实验室".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }

    #[test]
    fn keyword_query_requires_all_keywords() -> Result<(), SearchIndexError> {
        let schema = build_schema(SearchStrategy::Jieba);
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_tokenizers(&index, SearchStrategy::Jieba)?;

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "Rust 语言",
            content => "只有 Rust"
        ))?;
        writer.add_document(doc!(
            title => "Rust 搜索",
            content => "Rust 搜索 都有"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["Rust".to_string(), "搜索".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query, SearchStrategy::Jieba)?;
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
