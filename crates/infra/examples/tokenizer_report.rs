use std::fs;
use std::path::{Path, PathBuf};

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use serde::Deserialize;
use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, Stemmer, TextAnalyzer, TokenStream};

#[derive(Debug, Deserialize)]
struct SearchIndexEntry {
    title: String,
    subtitle: Option<String>,
    content: String,
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
enum SampleKind {
    Title,
    Subtitle,
    Content,
}

impl SampleKind {
    fn as_str(self) -> &'static str {
        match self {
            SampleKind::Title => "title",
            SampleKind::Subtitle => "subtitle",
            SampleKind::Content => "content",
        }
    }
}

#[derive(Debug)]
struct Sample {
    entry_index: usize,
    kind: SampleKind,
    text: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root_dir = repo_root()?;
    let corpus_path = root_dir.join("search-index.json");
    let samples = load_samples(&corpus_path, 160)?;
    let report_path = root_dir.join("reports").join("tokenizer_report.md");

    let mut jieba_analyzer = build_jieba_analyzer();
    let mut lindera_analyzer = build_lindera_analyzer();

    let mut same = 0usize;
    let mut diff = 0usize;
    let mut by_kind = std::collections::HashMap::<SampleKind, usize>::new();

    let mut report = String::new();
    report.push_str("# Tokenizer Report\n\n");
    report.push_str(&format!(
        "- corpus: `{}`\n",
        corpus_path.display()
    ));
    report.push_str("- content snippet length: 160 chars\n");
    report.push_str("- lindera dictionary: embedded://cc-cedict\n\n");

    for (idx, sample) in samples.iter().enumerate() {
        let jieba_tokens = tokenize_with_analyzer(&mut jieba_analyzer, &sample.text);
        let lindera_tokens = tokenize_with_analyzer(&mut lindera_analyzer, &sample.text);
        let is_same = jieba_tokens == lindera_tokens;
        if is_same {
            same += 1;
        } else {
            diff += 1;
        }
        *by_kind.entry(sample.kind).or_insert(0) += 1;

        report.push_str(&format!(
            "## Sample {idx}\n\n- entry_index: {}\n- kind: {}\n- same: {}\n\n",
            sample.entry_index,
            sample.kind.as_str(),
            is_same
        ));
        report.push_str("Text:\n\n```\n");
        report.push_str(&sample.text);
        report.push_str("\n```\n\n");
        report.push_str("Jieba tokens:\n\n```\n");
        report.push_str(&serde_json::to_string(&jieba_tokens)?);
        report.push_str("\n```\n\n");
        report.push_str("Lindera tokens:\n\n```\n");
        report.push_str(&serde_json::to_string(&lindera_tokens)?);
        report.push_str("\n```\n\n");
    }

    report.push_str("## Summary\n\n");
    report.push_str(&format!(
        "- total_samples: {}\n- same: {}\n- diff: {}\n",
        samples.len(),
        same,
        diff
    ));
    for (kind, count) in by_kind {
        report.push_str(&format!("- {kind}: {count}\n", kind = kind.as_str()));
    }

    fs::create_dir_all(report_path.parent().unwrap())?;
    fs::write(&report_path, report)?;
    println!("report written to {}", report_path.display());
    Ok(())
}

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    Ok(Path::new(&manifest_dir).join("..").join(".."))
}

fn build_jieba_analyzer() -> TextAnalyzer {
    let tokenizer = tantivy_jieba::JiebaTokenizer {};
    TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build()
}

fn build_lindera_analyzer() -> TextAnalyzer {
    let dictionary = load_dictionary("embedded://cc-cedict")
        .expect("load cc-cedict dictionary");
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    TextAnalyzer::builder(tokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build()
}

fn load_samples(
    path: &Path,
    content_snippet_len: usize,
) -> Result<Vec<Sample>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let entries: Vec<SearchIndexEntry> = serde_json::from_str(&content)?;
    let mut samples = Vec::new();
    for (idx, entry) in entries.into_iter().enumerate() {
        if !entry.title.trim().is_empty() {
            samples.push(Sample {
                entry_index: idx,
                kind: SampleKind::Title,
                text: entry.title,
            });
        }
        if let Some(subtitle) = entry.subtitle {
            if !subtitle.trim().is_empty() {
                samples.push(Sample {
                    entry_index: idx,
                    kind: SampleKind::Subtitle,
                    text: subtitle,
                });
            }
        }
        if !entry.content.trim().is_empty() {
            let snippet = entry
                .content
                .chars()
                .take(content_snippet_len)
                .collect::<String>();
            samples.push(Sample {
                entry_index: idx,
                kind: SampleKind::Content,
                text: snippet,
            });
        }
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
