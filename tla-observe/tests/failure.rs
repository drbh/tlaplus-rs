use std::{num::NonZeroUsize, ops::ControlFlow};
use tlaplus_engine::{Completion, Options, Transition, TransitionSystem, explore, explore_parallel};
use tlaplus_observe::{Diagnosed, Phase};

struct Model(Phase);
impl Model {
    fn checked<R>(&self, phase: Phase, value: R) -> Result<R, &'static str> {
        if self.0 == phase {
            Err("sentinel")
        } else {
            Ok(value)
        }
    }
}
impl TransitionSystem for Model {
    type State = usize;
    type Key = usize;
    type Error = &'static str;
    fn prepare_parallel(&self, _: NonZeroUsize) -> Result<(), Self::Error> {
        self.checked(Phase::PrepareParallel, ())
    }
    fn initial_states(&self) -> Result<Vec<usize>, Self::Error> {
        self.checked(Phase::InitialStates, vec![0])
    }
    fn successors(&self, state: &usize) -> Result<Vec<Transition<usize>>, Self::Error> {
        self.checked(
            Phase::Successors,
            vec![Transition {
                action: 0,
                state: (state + 1).min(1),
            }],
        )
    }
    fn key(&self, state: &usize) -> Result<usize, Self::Error> {
        self.checked(Phase::Canonicalization, *state)
    }
    fn same_state(&self, a: &usize, b: &usize) -> Result<bool, Self::Error> {
        self.checked(Phase::SameState, a == b)
    }
    fn state_allowed(&self, _: &usize) -> Result<bool, Self::Error> {
        self.checked(Phase::StateConstraint, true)
    }
    fn step_allowed(&self, _: &usize, _: &usize) -> Result<bool, Self::Error> {
        self.checked(Phase::ActionConstraint, true)
    }
    fn invariant(&self, _: &usize) -> Result<Option<String>, Self::Error> {
        self.checked(Phase::Invariant, None)
    }
    fn initial_property(&self, _: &usize) -> Result<Option<String>, Self::Error> {
        self.checked(Phase::InitialProperty, None)
    }
    fn action_property(&self, _: &usize, _: &usize) -> Result<Option<String>, Self::Error> {
        self.checked(Phase::ActionProperty, None)
    }
}

#[test]
fn errors_keep_the_originating_hook_and_state_in_both_engines() {
    for phase in [
        Phase::InitialStates,
        Phase::Successors,
        Phase::Canonicalization,
        Phase::StateConstraint,
        Phase::ActionConstraint,
        Phase::Invariant,
        Phase::InitialProperty,
        Phase::ActionProperty,
    ] {
        let model = Model(phase);
        let wrapped = Diagnosed(&model);
        let serial = explore(&wrapped, &Options::default()).unwrap_err();
        let parallel =
            explore_parallel(&wrapped, &Options::default(), NonZeroUsize::new(4).unwrap())
                .unwrap_err();
        for error in [serial, parallel] {
            assert_eq!(error.phase, phase);
            assert_eq!(error.error, "sentinel");
            assert_eq!(error.state, (phase != Phase::InitialStates).then_some(0));
            assert_eq!(
                error.next,
                matches!(phase, Phase::ActionConstraint | Phase::ActionProperty).then_some(1)
            );
        }
    }
    let model = Model(Phase::PrepareParallel);
    let error = explore_parallel(
        &Diagnosed(&model),
        &Options::default(),
        NonZeroUsize::new(4).unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.phase, Phase::PrepareParallel);
    assert_eq!(error.state, None);
    let model = Model(Phase::SameState);
    let error = Diagnosed(&model).same_state(&1, &2).unwrap_err();
    assert_eq!(
        (error.phase, error.state, error.next),
        (Phase::SameState, Some(1), Some(2))
    );
}

struct Streaming;
impl TransitionSystem for Streaming {
    type State = usize;
    type Key = usize;
    type Error = &'static str;
    fn initial_states(&self) -> Result<Vec<usize>, Self::Error> {
        panic!("stream required")
    }
    fn successors(&self, _: &usize) -> Result<Vec<Transition<usize>>, Self::Error> {
        Ok(vec![])
    }
    fn visit_initial(
        &self,
        out: &mut impl FnMut(usize) -> Result<ControlFlow<()>, Self::Error>,
    ) -> Result<ControlFlow<()>, Self::Error> {
        if out(0)?.is_break() {
            return Ok(ControlFlow::Break(()));
        }
        Err("later generation failure")
    }
    fn key(&self, state: &usize) -> Result<usize, Self::Error> {
        Ok(*state)
    }
    fn same_state(&self, a: &usize, b: &usize) -> Result<bool, Self::Error> {
        Ok(a == b)
    }
    fn invariant(&self, _: &usize) -> Result<Option<String>, Self::Error> {
        Ok(Some("Inv".into()))
    }
}

#[test]
fn diagnostics_preserve_streaming_termination() {
    let report = explore(&Diagnosed(&Streaming), &Options::default()).unwrap();
    assert!(matches!(report.completion, Completion::Violated(_)));
}
