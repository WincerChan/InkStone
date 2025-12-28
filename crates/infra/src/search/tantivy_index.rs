use std::path::Path;

use chrono::{DateTime, Utc};
use inkstone_core::domain::search::{SearchDocument, SearchHit, SearchQuery, SearchResult};
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
use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, Stemmer, TextAnalyzer};
use tantivy::{DocAddress, Index, IndexReader, Order, ReloadPolicy, TantivyDocument, Term};
use thiserror::Error;

use super::SearchSort;

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
    content: Field,
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

impl SearchIndex {
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Self, SearchIndexError> {
        let dir = path.as_ref();
        std::fs::create_dir_all(dir)?;

        let schema = build_schema();
        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir)?
        } else {
            Index::create_in_dir(dir, schema)?
        };
        register_jieba_tokenizer(&index);
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
        let (title_snippet, content_snippet) = if let Some(keyword_query) = built_query.keyword.as_ref()
        {
            let mut title_snippet =
                SnippetGenerator::create(&searcher, &**keyword_query, self.fields.title)?;
            title_snippet.set_max_num_chars(240);
            let mut content_snippet =
                SnippetGenerator::create(&searcher, &**keyword_query, self.fields.content)?;
            content_snippet.set_max_num_chars(240);
            (Some(title_snippet), Some(content_snippet))
        } else {
            (None, None)
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
                content_snippet.as_ref(),
            )?);
        }

        Ok(SearchResult { total, hits })
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
        document
    }

    fn document_to_hit(
        &self,
        doc: &TantivyDocument,
        title_snippet: Option<&SnippetGenerator>,
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
            content: snippet_or_excerpt(content_snippet, doc, self.fields.content, 120),
            url,
            tags,
            category,
            published_at: timestamp_to_datetime(published, "published")?,
            updated_at: timestamp_to_datetime(updated, "updated")?,
        })
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
            content: schema
                .get_field("content")
                .map_err(|_| SearchIndexError::MissingField("content"))?,
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
    builder.add_text_field("title", jieba_text_options(true));
    builder.add_text_field("content", jieba_text_options(true));
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("tags", STRING | STORED);
    builder.add_text_field("category", STRING | STORED);
    builder.add_i64_field("published", STORED | FAST);
    builder.add_i64_field("updated", STORED | FAST);
    builder.add_text_field("checksum", STRING | STORED);
    builder.build()
}

fn jieba_text_options(stored: bool) -> TextOptions {
    let indexing = TextFieldIndexing::default()
        .set_tokenizer("jieba")
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let options = TextOptions::default().set_indexing_options(indexing);
    if stored {
        options.set_stored()
    } else {
        options
    }
}

fn register_jieba_tokenizer(index: &Index) {
    let tokenizer = tantivy_jieba::JiebaTokenizer {};
    let analyzer = TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build();
    index.tokenizers().register("jieba", analyzer);
}

fn build_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<BuiltQuery, SearchIndexError> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    let keyword_query = build_keyword_query(index, fields, query)?;
    if let Some(keyword_query) = keyword_query.as_ref() {
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

    let query: Box<dyn Query> = if clauses.is_empty() {
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::from(clauses))
    };
    Ok(BuiltQuery { query, keyword: keyword_query })
}

struct BuiltQuery {
    query: Box<dyn Query>,
    keyword: Option<Box<dyn Query>>,
}

fn build_keyword_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<Option<Box<dyn Query>>, SearchIndexError> {
    if query.keywords.is_empty() {
        return Ok(None);
    }
    let mut analyzer = index
        .tokenizers()
        .get("jieba")
        .ok_or(SearchIndexError::MissingTokenizer("jieba"))?;
    let mut keyword_queries = Vec::new();
    for keyword in &query.keywords {
        let tokens = tokenize_keyword(&mut analyzer, keyword);
        let title_query = build_field_query(fields.title, &tokens);
        let content_query = build_field_query(fields.content, &tokens);
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        if let Some(query) = title_query {
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
                .map(|query| (Occur::Should, query))
                .collect(),
        ))))
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
    use tantivy::collector::TopDocs;
    use tantivy::doc;
    use tantivy::tokenizer::TokenStream;

    #[test]
    fn jieba_tokenizer_searches_chinese() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "张华考上了北京大学；我在百货公司当售货员",
            content => "百货公司里有一个售货员正在忙碌。"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
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
        let schema = build_schema();
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

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
    fn jieba_searches_three_years_phrase_in_title() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "离职，三年未满",
            content => "正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["三年".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let top_docs = searcher.search(&built.query, &TopDocs::with_limit(5))?;
        assert!(!top_docs.is_empty());
        Ok(())
    }

    #[test]
    fn title_snippet_falls_back_to_title_text() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "离职，三年未满",
            content => "正文"
        ))?;
        writer.commit()?;

        let fields = SearchFields::from_schema(&index.schema())?;
        let searcher = index.reader()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["正文".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
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
    fn keyword_query_matches_tags() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let tags = schema.get_field("tags")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            content => "无关正文",
            tags => "售货员"
        ))?;
        writer.commit()?;

        let fields = SearchFields {
            id: index.schema().get_field("id")?,
            title,
            content,
            url: index.schema().get_field("url")?,
            tags,
            category: index.schema().get_field("category")?,
            published: index.schema().get_field("published")?,
            updated: index.schema().get_field("updated")?,
            checksum: index.schema().get_field("checksum")?,
        };

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["售货员".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }

    #[test]
    fn range_query_matches_updated() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let published = schema.get_field("published")?;
        let updated = schema.get_field("updated")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "只更新命中",
            content => "正文内容",
            published => 1_577_836_800i64,
            updated => 1_735_689_600i64
        ))?;
        writer.commit()?;

        let fields = SearchFields {
            id: index.schema().get_field("id")?,
            title,
            content,
            url: index.schema().get_field("url")?,
            tags: index.schema().get_field("tags")?,
            category: index.schema().get_field("category")?,
            published,
            updated,
            checksum: index.schema().get_field("checksum")?,
        };

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
    fn keyword_query_matches_category() -> Result<(), SearchIndexError> {
        let schema = build_schema();
        let title = schema.get_field("title")?;
        let content = schema.get_field("content")?;
        let category = schema.get_field("category")?;
        let index = Index::create_in_ram(schema);
        register_jieba_tokenizer(&index);

        let mut writer = index.writer::<TantivyDocument>(50_000_000)?;
        writer.add_document(doc!(
            title => "无关标题",
            content => "无关正文",
            category => "实验室"
        ))?;
        writer.commit()?;

        let fields = SearchFields {
            id: index.schema().get_field("id")?,
            title,
            content,
            url: index.schema().get_field("url")?,
            tags: index.schema().get_field("tags")?,
            category,
            published: index.schema().get_field("published")?,
            updated: index.schema().get_field("updated")?,
            checksum: index.schema().get_field("checksum")?,
        };

        let searcher = index.reader_builder().try_into()?.searcher();
        let search_query = SearchQuery {
            keywords: vec!["实验室".to_string()],
            ..Default::default()
        };
        let built = build_query(&index, &fields, &search_query)?;
        let hits = searcher.search(&built.query, &TopDocs::with_limit(10))?;
        assert_eq!(hits.len(), 1);
        Ok(())
    }
}
