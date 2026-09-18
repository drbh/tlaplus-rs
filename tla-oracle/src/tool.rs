//! The two TLC message encodings: framed CLI text and JUnit recorder parameters.

use serde::{Deserialize, Serialize};

use crate::{Answers, Error, Mode, Outcome};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub code: i32,
    pub severity: i32,
    pub text: String,
}

/// Parse `-tool` framing, allowing ordinary SANY/Print output between messages.
/// Damaged or truncated frames are errors, never silently discarded evidence.
pub fn parse_stream(input: &str) -> Result<Vec<Message>, Error> {
    let normalized = input.replace("\r\n", "\n");
    let mut messages = Vec::new();
    let mut current: Option<Message> = None;
    let mut embedded = Vec::new();
    for (line_index, line) in normalized.lines().enumerate() {
        let error = |detail| Error::Parse(format!("tool stream line {}: {detail}", line_index + 1));
        // Exceptions can quote another framed diagnostic, including an inline
        // `: STARTMSG`. Preserve that entire frame as the outer message's body.
        let header = if current.is_some() {
            line.split_once("@!@!@STARTMSG ").map(|(_, header)| header)
        } else {
            line.strip_prefix("@!@!@STARTMSG ")
        };
        if let Some(header) = header {
            let (code, severity) = header
                .strip_suffix(" @!@!@")
                .and_then(|s| s.split_once(':'))
                .ok_or_else(|| error("malformed STARTMSG"))?;
            let code = code.parse().map_err(|_| error("invalid message code"))?;
            let severity = severity.parse().map_err(|_| error("invalid severity"))?;
            if current.is_some() {
                embedded.push(code);
            } else {
                current = Some(Message {
                    code,
                    severity,
                    text: String::new(),
                });
                continue;
            }
        } else if let Some(footer) = line.strip_prefix("@!@!@ENDMSG ") {
            let code: i32 = footer
                .strip_suffix(" @!@!@")
                .ok_or_else(|| error("malformed ENDMSG"))?
                .parse()
                .map_err(|_| error("invalid ENDMSG code"))?;
            if let Some(expected) = embedded.pop() {
                if code != expected {
                    return Err(error("embedded STARTMSG/ENDMSG codes differ"));
                }
            } else {
                let mut message = current.take().ok_or_else(|| error("unmatched ENDMSG"))?;
                if message.code != code {
                    return Err(error("STARTMSG/ENDMSG codes differ"));
                }
                message.text.pop();
                messages.push(message);
                continue;
            }
        }
        if let Some(message) = &mut current {
            message.text.push_str(line);
            message.text.push('\n');
        }
    }
    if let Some(message) = current {
        return Err(Error::Parse(format!(
            "unterminated tool message {}",
            message.code
        )));
    }
    Ok(messages)
}

fn number(text: &str) -> Result<u64, Error> {
    let groups: Vec<_> = text.split(',').collect();
    if groups
        .iter()
        .any(|g| g.is_empty() || !g.bytes().all(|b| b.is_ascii_digit()))
        || (groups.len() > 1 && (groups[0].len() > 3 || groups[1..].iter().any(|g| g.len() != 3)))
    {
        return Err(Error::Parse(format!("invalid TLC count: {text:?}")));
    }
    groups
        .concat()
        .parse()
        .map_err(|_| Error::Parse(format!("TLC count overflows u64: {text}")))
}

fn between<'a>(text: &'a str, prefix: &str, suffix: &str) -> Result<&'a str, Error> {
    text.strip_prefix(prefix)
        .and_then(|s| s.split_once(suffix).map(|(value, _)| value))
        .ok_or_else(|| Error::Parse(format!("malformed TLC answer: {text:?}")))
}

#[derive(Default)]
struct Extractor {
    answers: Answers,
    success: bool,
    finished: bool,
    // A success marker can precede a late I/O error. Do not turn it into a pass.
    unclassified_error: bool,
}

impl Extractor {
    fn code(&mut self, code: i32) {
        match code {
            code::TLC_MODE_MC => self.answers.mode = Mode::Bfs,
            code::TLC_MODE_MC_DFS => self.answers.mode = Mode::Dfid,
            code::TLC_MODE_SIMU => self.answers.mode = Mode::Simulation,
            code::TLC_SUCCESS => self.success = true,
            code::TLC_FINISHED => self.finished = true,
            code::TLC_PARSING_FAILED | code::TLC_PARSING_FAILED2 => {
                self.answers.outcome = Outcome::Rejected
            }
            code::TLC_ASSUMPTION_FALSE => self.answers.outcome = Outcome::AssumptionViolated,
            code::TLC_INVARIANT_VIOLATED_INITIAL | code::TLC_INVARIANT_VIOLATED_BEHAVIOR => {
                self.answers.outcome = Outcome::InvariantViolated;
            }
            code::TLC_PROPERTY_VIOLATED_INITIAL => {
                self.answers.outcome = Outcome::InitialPropertyViolated
            }
            code::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR => {
                self.answers.outcome = Outcome::ActionPropertyViolated
            }
            code::TLC_TEMPORAL_PROPERTY_VIOLATED => {
                self.answers.outcome = Outcome::TemporalPropertyViolated
            }
            code::TLC_DEADLOCK_REACHED => self.answers.outcome = Outcome::Deadlock,
            _ => {}
        }
    }

    fn finish(mut self, exit_status: Option<i32>) -> Answers {
        if self.answers.outcome == Outcome::Unknown
            && self.success
            && self.finished
            && exit_status == Some(0)
            && !self.unclassified_error
        {
            self.answers.outcome = Outcome::Success;
        }
        if exit_status.is_none_or(|status| status < 0) {
            self.answers.outcome = Outcome::Unknown;
        }
        self.answers
    }
}

pub fn extract_messages(messages: &[Message], exit_status: Option<i32>) -> Result<Answers, Error> {
    let mut extractor = Extractor::default();
    for message in messages {
        extractor.code(message.code);
        if message.severity == 1 {
            extractor.unclassified_error = true;
        }
        let text = message.text.trim();
        match message.code {
            code::TLC_INIT_GENERATED1 => {
                extractor.answers.initial_distinct = Some(number(between(
                    text,
                    "Finished computing initial states: ",
                    " distinct state",
                )?)?);
            }
            code::TLC_INIT_GENERATED2 | code::TLC_INIT_GENERATED4 => {
                let (_, rest) = text
                    .split_once(" generated, with ")
                    .ok_or_else(|| Error::Parse(format!("malformed initial counts: {text:?}")))?;
                extractor.answers.initial_distinct =
                    Some(number(between(rest, "", " of them distinct")?)?);
            }
            code::TLC_STATS => {
                let (_, rest) = text
                    .split_once(" states generated, ")
                    .ok_or_else(|| Error::Parse(format!("malformed final counts: {text:?}")))?;
                extractor.answers.distinct =
                    Some(number(between(rest, "", " distinct states found,")?)?);
            }
            code::TLC_SEARCH_DEPTH => {
                extractor.answers.depth = Some(number(between(
                    text,
                    "The depth of the complete state graph search is ",
                    ".",
                )?)?);
            }
            _ => {}
        }
    }
    Ok(extractor.finish(exit_status))
}

/// Recorder parameters are not CLI templates (notably, trace parameters differ).
/// Extract counts directly so this adapter is independent of text rendering.
pub fn extract_records<'a>(
    records: impl IntoIterator<Item = (i32, &'a [String])>,
    exit_status: Option<i32>,
) -> Result<Answers, Error> {
    let mut extractor = Extractor::default();
    for (code, params) in records {
        extractor.code(code);
        let index = match code {
            code::TLC_INIT_GENERATED1 | code::TLC_SEARCH_DEPTH => 0,
            code::TLC_INIT_GENERATED2 => 2,
            code::TLC_INIT_GENERATED4 | code::TLC_STATS => 1,
            _ => continue,
        };
        let value = number(params.get(index).ok_or_else(|| {
            Error::Parse(format!("missing parameter {index} for TLC message {code}"))
        })?)?;
        match code {
            code::TLC_SEARCH_DEPTH => extractor.answers.depth = Some(value),
            code::TLC_STATS => extractor.answers.distinct = Some(value),
            _ => extractor.answers.initial_distinct = Some(value),
        }
    }
    Ok(extractor.finish(exit_status))
}

pub mod code {
    pub const TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR: i32 = 2112;
    pub const TLC_ASSUMPTION_FALSE: i32 = 2104;
    pub const TLC_DEADLOCK_REACHED: i32 = 2114;
    pub const TLC_FINISHED: i32 = 2186;
    pub const TLC_INIT_GENERATED1: i32 = 2190;
    pub const TLC_INIT_GENERATED2: i32 = 2191;
    pub const TLC_INIT_GENERATED4: i32 = 2208;
    pub const TLC_INVARIANT_EVALUATION_FAILED: i32 = 2111;
    pub const TLC_INVARIANT_VIOLATED_BEHAVIOR: i32 = 2110;
    pub const TLC_INVARIANT_VIOLATED_INITIAL: i32 = 2107;
    pub const TLC_MODE_MC: i32 = 2187;
    pub const TLC_MODE_MC_DFS: i32 = 2271;
    pub const TLC_MODE_SIMU: i32 = 2188;
    pub const TLC_PARSING_FAILED: i32 = 3002;
    pub const TLC_PARSING_FAILED2: i32 = 2171;
    pub const TLC_PROGRESS_STATS: i32 = 2200;
    pub const TLC_PROPERTY_VIOLATED_INITIAL: i32 = 2108;
    pub const TLC_SEARCH_DEPTH: i32 = 2194;
    pub const TLC_STATS: i32 = 2199;
    pub const TLC_SUCCESS: i32 = 2193;
    pub const TLC_TEMPORAL_PROPERTY_VIOLATED: i32 = 2116;
    pub const TLC_VERSION: i32 = 2262;
}
