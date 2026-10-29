use tlaplus_oracle::tool::code as mp;
use tlaplus_oracle::tool::{Message, extract_messages, extract_records, parse_stream};
use tlaplus_oracle::{Answers, Comparison, Difference, Mode, Outcome, compare};

fn message(code: i32, text: &str) -> Message {
    Message {
        code,
        severity: 0,
        text: text.into(),
    }
}

fn success_messages() -> Vec<Message> {
    vec![
        message(mp::TLC_MODE_MC, "machine-dependent banner"),
        message(
            mp::TLC_INIT_GENERATED2,
            "Finished computing initial states: 9 states generated, with 2 of them distinct at 2026-09-25 15:11:33.",
        ),
        message(mp::TLC_SUCCESS, "success"),
        message(
            mp::TLC_STATS,
            "999,999 states generated, 1,234 distinct states found, 0 states left on queue.",
        ),
        message(
            mp::TLC_SEARCH_DEPTH,
            "The depth of the complete state graph search is 42.",
        ),
        message(mp::TLC_FINISHED, "time-dependent finish"),
    ]
}

#[test]
fn normalizes_counts_and_ignores_non_contract_messages() {
    let mut messages = success_messages();
    messages.insert(
        1,
        message(mp::TLC_PROGRESS_STATS, "Progress(99): 999999 states"),
    );
    messages.push(message(mp::TLC_VERSION, "some future version"));
    let answer = extract_messages(&messages, Some(0)).unwrap();
    assert_eq!(
        answer,
        Answers {
            mode: Mode::Bfs,
            outcome: Outcome::Success,
            initial_distinct: Some(2),
            distinct: Some(1234),
            depth: Some(42),
        }
    );
    let records = [
        (mp::TLC_MODE_MC, vec![]),
        (mp::TLC_INIT_GENERATED2, vec!["9", "s", "2"]),
        (mp::TLC_SUCCESS, vec![]),
        (mp::TLC_STATS, vec!["7", "1234", "0"]),
        (mp::TLC_SEARCH_DEPTH, vec!["42"]),
        (mp::TLC_FINISHED, vec!["0ms"]),
    ]
    .into_iter()
    .map(|(code, params)| {
        (
            code,
            params.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    })
    .collect::<Vec<_>>();
    assert_eq!(
        answer,
        extract_records(
            records
                .iter()
                .map(|(code, params)| (*code, params.as_slice())),
            Some(0)
        )
        .unwrap()
    );
}

#[test]
fn parses_framing_multiline_crlf_and_external_noise() {
    let input = "SANY text\r\n@!@!@STARTMSG 2193:0 @!@!@\r\nfirst\r\n\r\nlast\r\n@!@!@ENDMSG 2193 @!@!@\r\nPrint output\r\n";
    assert_eq!(
        parse_stream(input).unwrap(),
        vec![message(2193, "first\n\nlast")]
    );
    for bad in [
        "@!@!@STARTMSG 2193:0 @!@!@\nunfinished",
        "@!@!@STARTMSG x:0 @!@!@\n",
        "@!@!@STARTMSG 2193:0 @!@!@\n@!@!@ENDMSG 2194 @!@!@",
        "@!@!@ENDMSG 2193 @!@!@",
        "@!@!@STARTMSG 2193:0 @!@!@\n@!@!@STARTMSG 2194:0 @!@!@",
    ] {
        assert!(parse_stream(bad).is_err(), "accepted {bad:?}");
    }
}

#[test]
fn incomplete_or_failed_runs_cannot_pass() {
    for status in [None, Some(-9), Some(1)] {
        assert_eq!(
            extract_messages(&success_messages(), status)
                .unwrap()
                .outcome,
            Outcome::Unknown
        );
    }
    let mut messages = success_messages();
    messages.pop();
    assert_eq!(
        extract_messages(&messages, Some(0)).unwrap().outcome,
        Outcome::Unknown
    );
    assert_eq!(
        extract_messages(&[], Some(0)).unwrap().outcome,
        Outcome::Unknown
    );
    messages = success_messages();
    messages.push(Message {
        code: 9999,
        severity: 1,
        text: "new error".into(),
    });
    assert_eq!(
        extract_messages(&messages, Some(0)).unwrap().outcome,
        Outcome::Unknown
    );
    assert_eq!(
        compare(&Answers::default(), &Answers::default()),
        Comparison::Inconclusive
    );
}

#[test]
fn errors_do_not_become_zero_counts() {
    for count in ["", "1,23", "-1", "18446744073709551616", "NaN"] {
        let message = message(
            mp::TLC_SEARCH_DEPTH,
            &format!("The depth of the complete state graph search is {count}."),
        );
        assert!(extract_messages(&[message], Some(0)).is_err());
    }
    assert!(extract_records([(mp::TLC_STATS, [].as_slice())], Some(0)).is_err());
}

#[test]
fn classifies_explicit_verdicts_without_treating_all_errors_as_violations() {
    for (code, outcome) in [
        (mp::TLC_PARSING_FAILED, Outcome::Rejected),
        (mp::TLC_PARSING_FAILED2, Outcome::Rejected),
        (mp::TLC_ASSUMPTION_FALSE, Outcome::AssumptionViolated),
        (
            mp::TLC_INVARIANT_VIOLATED_INITIAL,
            Outcome::InvariantViolated,
        ),
        (
            mp::TLC_INVARIANT_VIOLATED_BEHAVIOR,
            Outcome::InvariantViolated,
        ),
        (
            mp::TLC_PROPERTY_VIOLATED_INITIAL,
            Outcome::InitialPropertyViolated,
        ),
        (
            mp::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR,
            Outcome::ActionPropertyViolated,
        ),
        (
            mp::TLC_TEMPORAL_PROPERTY_VIOLATED,
            Outcome::TemporalPropertyViolated,
        ),
        (mp::TLC_DEADLOCK_REACHED, Outcome::Deadlock),
        (mp::TLC_INVARIANT_EVALUATION_FAILED, Outcome::Unknown),
    ] {
        let messages = [message(code, "diagnostic text is not the verdict")];
        let records = [(code, [].as_slice())];
        assert_eq!(
            extract_messages(&messages, Some(255)).unwrap().outcome,
            outcome
        );
        assert_eq!(
            extract_records(records, Some(255)).unwrap().outcome,
            outcome
        );
    }
}

#[test]
fn comparison_checks_completed_counts_but_not_early_stop_counts() {
    let left = extract_messages(&success_messages(), Some(0)).unwrap();
    let mut right = left.clone();
    right.depth = Some(41);
    right.distinct = Some(1233);
    assert_eq!(
        compare(&left, &right),
        Comparison::Different(vec![Difference::Distinct, Difference::Depth])
    );
    right = left.clone();
    right.depth = None;
    assert_eq!(compare(&left, &right), Comparison::Inconclusive);
    let mut left = left;
    left.outcome = Outcome::InvariantViolated;
    right.outcome = Outcome::InvariantViolated;
    right.distinct = Some(10);
    assert_eq!(compare(&left, &right), Comparison::Match);
    right.outcome = Outcome::Deadlock;
    assert_eq!(
        compare(&left, &right),
        Comparison::Different(vec![Difference::Outcome])
    );
    for mode in [Mode::Simulation, Mode::Dfid] {
        left.mode = mode;
        assert_eq!(compare(&left, &left), Comparison::Inconclusive);
    }
}
