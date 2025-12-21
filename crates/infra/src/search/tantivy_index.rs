use std::path::Path;

use chrono::{DateTime, Utc};
use inkstone_core::domain::search::{SearchDocument, SearchHit, SearchQuery, SearchResult};
use std::ops::Bound;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, SchemaBuilder, Value, FAST, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, ReloadPolicy, TantivyDocument, Term};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchIndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("query parse error: {0}")]
    QueryParse(#[from] tantivy::query::QueryParserError),
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
    summary: Field,
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
    ) -> Result<SearchResult, SearchIndexError> {
        let searcher = self.reader.searcher();
        let tantivy_query = build_query(&self.index, &self.fields, query)?;
        let total = searcher.search(&tantivy_query, &Count)?;
        let docs = searcher.search(
            &tantivy_query,
            &TopDocs::with_limit(limit.saturating_add(offset)),
        )?;

        let mut hits = Vec::new();
        for (_, address) in docs.into_iter().skip(offset).take(limit) {
            let doc: TantivyDocument = searcher.doc(address)?;
            hits.push(self.document_to_hit(&doc)?);
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
        if let Some(summary) = &doc.summary {
            document.add_text(self.fields.summary, summary);
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
        document
    }

    fn document_to_hit(&self, doc: &TantivyDocument) -> Result<SearchHit, SearchIndexError> {
        let id = get_string(doc, self.fields.id).ok_or(SearchIndexError::MissingValue("id"))?;
        let title = get_string(doc, self.fields.title).ok_or(SearchIndexError::MissingValue("title"))?;
        let summary = get_string(doc, self.fields.summary);
        let url = get_string(doc, self.fields.url).ok_or(SearchIndexError::MissingValue("url"))?;
        let tags = get_strings(doc, self.fields.tags);
        let category = get_string(doc, self.fields.category);
        let published = get_i64(doc, self.fields.published)
            .ok_or(SearchIndexError::MissingValue("published"))?;
        let updated = get_i64(doc, self.fields.updated)
            .ok_or(SearchIndexError::MissingValue("updated"))?;

        Ok(SearchHit {
            id,
            title,
            summary,
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
            summary: schema
                .get_field("summary")
                .map_err(|_| SearchIndexError::MissingField("summary"))?,
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
    builder.add_text_field("title", TEXT | STORED);
    builder.add_text_field("summary", TEXT | STORED);
    builder.add_text_field("content", TEXT);
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("tags", STRING | STORED);
    builder.add_text_field("category", STRING | STORED);
    builder.add_i64_field("published", STORED | FAST);
    builder.add_i64_field("updated", STORED | FAST);
    builder.add_text_field("checksum", STRING | STORED);
    builder.build()
}

fn build_query(
    index: &Index,
    fields: &SearchFields,
    query: &SearchQuery,
) -> Result<Box<dyn Query>, SearchIndexError> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

    if !query.keywords.is_empty() {
        let parser = QueryParser::for_index(index, vec![fields.title, fields.content, fields.summary]);
        let query_str = query.keywords.join(" ");
        let keyword_query = parser.parse_query(&query_str)?;
        clauses.push((Occur::Must, keyword_query));
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
        let range_query: Box<dyn Query> = match (start, end) {
            (Some(start), Some(end)) => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_i64(fields.published, start)),
                Bound::Included(Term::from_field_i64(fields.published, end)),
            )),
            (Some(start), None) => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_i64(fields.published, start)),
                Bound::Unbounded,
            )),
            (None, Some(end)) => Box::new(RangeQuery::new(
                Bound::Unbounded,
                Bound::Included(Term::from_field_i64(fields.published, end)),
            )),
            (None, None) => Box::new(AllQuery),
        };
        clauses.push((Occur::Must, range_query));
    }

    if clauses.is_empty() {
        Ok(Box::new(AllQuery))
    } else {
        Ok(Box::new(BooleanQuery::from(clauses)))
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

fn timestamp_to_datetime(ts: i64, field: &'static str) -> Result<DateTime<Utc>, SearchIndexError> {
    DateTime::<Utc>::from_timestamp(ts, 0).ok_or(SearchIndexError::InvalidTimestamp(field))
}
