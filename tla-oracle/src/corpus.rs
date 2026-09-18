//! Read existing oracle artifacts without modifying or regenerating them.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::tool::{Message, extract_messages, extract_records};
use crate::{Answers, Comparison, Difference, Error, compare};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Corpus {
    pub answers: BTreeMap<String, Answers>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub matched: usize,
    pub differences: BTreeMap<String, Vec<Difference>>,
    pub inconclusive: Vec<String>,
    pub missing: Vec<String>,
    pub unexpected: Vec<String>,
}

impl Report {
    pub fn is_match(&self) -> bool {
        self.matched > 0
            && self.differences.is_empty()
            && self.inconclusive.is_empty()
            && self.missing.is_empty()
            && self.unexpected.is_empty()
    }
}

pub fn compare_corpora(expected: &Corpus, actual: &Corpus) -> Report {
    let mut report = Report::default();
    for (id, left) in &expected.answers {
        match actual.answers.get(id) {
            None => report.missing.push(id.clone()),
            Some(right) => match compare(left, right) {
                Comparison::Match => report.matched += 1,
                Comparison::Different(fields) => {
                    report.differences.insert(id.clone(), fields);
                }
                Comparison::Inconclusive => report.inconclusive.push(id.clone()),
            },
        }
    }
    report.unexpected = actual
        .answers
        .keys()
        .filter(|id| !expected.answers.contains_key(*id))
        .cloned()
        .collect();
    report
}

#[derive(Deserialize)]
struct Example {
    cfg: String,
    #[serde(default)]
    messages: Vec<Message>,
    run: ExampleRun,
}

#[derive(Deserialize)]
struct ExampleRun {
    run_status: String,
    returncode: Option<i32>,
}

#[derive(Deserialize)]
struct Manifest {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    class: String,
    #[serde(default)]
    methods: Vec<Method>,
}

#[derive(Deserialize)]
struct Method {
    method: String,
    actual_exit_status: Option<i32>,
    #[serde(default)]
    records: Vec<RecordedMessage>,
}

#[derive(Deserialize)]
struct RecordedMessage {
    code: i32,
    params: Vec<Option<String>>,
}

impl Corpus {
    /// Load a JUnit manifest, an Examples recording, an extracted snapshot, or
    /// a directory of Examples recordings. Gzip is detected from the extension.
    pub fn read(path: &Path) -> Result<Self, Error> {
        let result = if path.is_dir() {
            let mut paths = fs::read_dir(path)?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()?;
            paths.sort();
            let mut corpus = Self::default();
            for path in paths.into_iter().filter(|p| {
                p.is_file() && matches!(p.extension().and_then(|s| s.to_str()), Some("json" | "gz"))
            }) {
                for (id, answers) in Self::read(&path)?.answers {
                    corpus.insert(id, answers)?;
                }
            }
            Ok(corpus)
        } else {
            Self::read_file(path)
        };
        let corpus = result.map_err(|e| Error::Corpus(format!("{}: {e}", path.display())))?;
        if corpus.answers.is_empty() {
            return Err(Error::Corpus(format!(
                "{}: no oracle cases found",
                path.display()
            )));
        }
        Ok(corpus)
    }

    fn insert(&mut self, id: String, answers: Answers) -> Result<(), Error> {
        if self.answers.contains_key(&id) {
            return Err(Error::Corpus(format!("duplicate oracle case: {id}")));
        }
        self.answers.insert(id, answers);
        Ok(())
    }

    fn read_file(path: &Path) -> Result<Self, Error> {
        let file = fs::File::open(path)?;
        let mut reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "gz") {
            Box::new(flate2::read::GzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let mut text = String::new();
        reader.read_to_string(&mut text)?;
        Self::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        let json: serde_json::Value = serde_json::from_str(text)?;
        if json.get("answers").is_some() {
            return Ok(serde_json::from_value(json)?);
        }
        let mut corpus = Self::default();
        if json.get("cases").is_some() {
            let manifest: Manifest = serde_json::from_value(json)?;
            for case in manifest.cases {
                for method in case.methods {
                    let id = format!("{}#{}", case.class, method.method);
                    let records: Vec<_> = method
                        .records
                        .into_iter()
                        .map(|r| {
                            (
                                r.code,
                                r.params
                                    .into_iter()
                                    .map(|p| p.unwrap_or_else(|| "null".into()))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect();
                    let answers = extract_records(
                        records
                            .iter()
                            .map(|(code, params)| (*code, params.as_slice())),
                        method.actual_exit_status,
                    )
                    .map_err(|e| Error::Corpus(format!("{id}: {e}")))?;
                    corpus.insert(id, answers)?;
                }
            }
        } else {
            let example: Example = serde_json::from_value(json)?;
            let exit = if example.run.run_status == "completed" {
                example.run.returncode
            } else {
                None
            };
            let answers = extract_messages(&example.messages, exit)?;
            corpus.insert(example.cfg, answers)?;
        }
        Ok(corpus)
    }
}
