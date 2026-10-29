//! Scoped faithful evaluation: raw values and states cannot outlive the JVM session.

pub use crate::ModelRequest;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use tla_tlc_eval::nextstate::{InitVec, NextVec};
use tla_tlc_eval::{ProcessedSpec, SetupOptions, TlcState, Tool, mp};
use tla_tlc_value::TlcError;

pub use tlaplus_oracle::{ModelError, Stage, StateSnapshot};

type Result<T> = std::result::Result<T, ModelError>;

pub use tlaplus_oracle::{ActionId, ActionInfo, ModelInfo};

pub struct Model {
    pub(crate) initial_context: RefCell<tla_tlc_eval::tool::RuntimeSnapshot>,
    pub(crate) initial_random: RefCell<tla_tlc_value::jrandom::JavaRandom>,
    pub(crate) replay_random: Cell<bool>,
    pub(crate) tool: Tool,
    pub(crate) processed: ProcessedSpec,
    info: ModelInfo,
}

/// An immutable, complete faithful state tied to its originating session.
/// Equality must use `same_values`: faithful `TlcState::eq` is pointer equality.
#[derive(Clone)]
pub struct State<'model> {
    pub(crate) model: &'model Model,
    pub(crate) raw: TlcState,
}

#[derive(Clone)]
pub struct Transition<'model> {
    pub action: ActionId,
    pub state: State<'model>,
}

thread_local! {
    static IN_SESSION: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn ensure_outside_session() -> Result<()> {
    if IN_SESSION.get() {
        Err(ModelError::message(
            Stage::Session,
            "an oracle session is already active on this thread; finish it before starting another",
        ))
    } else {
        Ok(())
    }
}

struct SessionGuard(mp::Recorder);

impl SessionGuard {
    fn start(seed: i64, polynomial: u64) -> Self {
        IN_SESSION.set(true);
        reset_controls();
        tla_tlc_value::fp64::init_with(polynomial);
        tla_tlc_value::random::set_seed(seed);
        Self(mp::Recorder::start())
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.stop();
        reset_controls();
        tla_tlc_eval::state::set_variables(Vec::new());
        IN_SESSION.set(false);
    }
}

fn reset_controls() {
    mp::set_printer(None);
    mp::set_tool_mode(false);
    mp::set_no_timestamps(false);
    mp::set_warnings_enabled(true);
    mp::set_message_controls(&[], &[], &[], &[]);
    tla_tlc_eval::modules::tlc::set_user_output(None);
    tla_tlc_eval::state::set_use_view(false);
    tla_tlc_eval::tool::set_current_worker(None);
    tla_tlc_value::random::set_current_state(None);
    tla_tlc_value::globals::set_expand(true);
    tla_tlc_value::globals::set_set_bound(1_000_000);
}

pub fn with_model<T: Send + 'static>(
    request: ModelRequest,
    inspect: impl FnOnce(&Model) -> Result<T> + Send + 'static,
) -> Result<T> {
    with_context(request, false, inspect)
}

pub fn with_register_model<T: Send + 'static>(
    request: ModelRequest,
    inspect: impl FnOnce(&Model) -> Result<T> + Send + 'static,
) -> Result<T> {
    with_context(request, true, inspect)
}

fn with_context<T: Send + 'static>(
    request: ModelRequest,
    registers: bool,
    inspect: impl FnOnce(&Model) -> Result<T> + Send + 'static,
) -> Result<T> {
    with_session(request, move |model, records| {
        if registers {
            model
                .tool
                .set_checker_registers(Some(tla_tlc_eval::tool::CheckerRegisters {
                    local: vec![Vec::new()],
                    named: vec![Vec::new()],
                    ..Default::default()
                }));
        }
        let code = model.tool.check_assumptions();
        if code != mp::NO_ERROR {
            let records = records.take();
            let params = records
                .iter()
                .rev()
                .find(|r| r.code == code)
                .map(|r| r.params.as_slice())
                .unwrap_or(&[]);
            return Err(ModelError {
                stage: Stage::Assumptions,
                code: Some(code),
                message: mp::get_message(code, params),
            });
        }
        model.capture_setup();
        inspect(model)
    })
}

/// Setup inspection before ASSUME checking, for differential validation only.
/// The caller must validate assumptions before exploring this model.
/// ```compile_fail
/// tlaplus_bridge::with_loaded_model(tlaplus_bridge::ModelRequest::new("Counter.tla"),
///     |model| model.initial_states());
/// ```
pub fn with_loaded_model<T: Send + 'static>(
    request: ModelRequest,
    inspect: impl FnOnce(&Model) -> Result<T> + Send + 'static,
) -> Result<T> {
    with_session(request, move |model, _| inspect(model))
}

fn with_session<T: Send + 'static>(
    request: ModelRequest,
    inspect: impl FnOnce(&Model, &mp::Recorder) -> Result<T> + Send + 'static,
) -> Result<T> {
    ensure_outside_session()?;
    let polynomial = *tla_tlc_value::fp64::POLYS
        .get(request.fingerprint_index)
        .ok_or_else(|| ModelError::message(Stage::Load, "invalid fingerprint index"))?;
    tla_tlc_eval::in_fresh_jvm(move || {
        let guard = SessionGuard::start(request.seed, polynomial);
        let model = match Model::load(request) {
            Ok(model) => model,
            Err(mut error) => {
                append_setup_diagnostics(&mut error, &guard.0.take());
                return Err(error);
            }
        };
        inspect(&model, &guard.0)
    })
}

impl Model {
    fn load(request: ModelRequest) -> Result<Self> {
        let root_file =
            std::path::absolute(&request.spec).map_err(|e| ModelError::message(Stage::Load, e))?;
        let root = root_file
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                ModelError::message(Stage::Load, "spec path needs a UTF-8 module name")
            })?;
        let directory = root_file
            .parent()
            .ok_or_else(|| ModelError::message(Stage::Load, "spec path needs a directory"))?;
        let config = request
            .config
            .map(|p| directory.join(p))
            .unwrap_or_else(|| root_file.with_extension("cfg"));
        let mut options = SetupOptions {
            seed: request.seed,
            generate_te_spec: false,
            library_paths: request
                .library_paths
                .into_iter()
                .map(|p| directory.join(p))
                .collect(),
            ..SetupOptions::default()
        };
        options.tool.community_modules = request.community_modules;
        let tool = tla_tlc_eval::setup(
            &[directory.to_path_buf()],
            &root_file,
            root,
            Some(&config),
            &options,
        )
        .map_err(|error| {
            let (code, message) = error.code_and_diagnostics();
            ModelError {
                stage: Stage::Load,
                code: Some(code),
                message,
            }
        })?;
        let processed = tool.processed_spec();
        let action_info = |action: &tla_tlc_eval::spec::Action| ActionInfo {
            id: ActionId(action.id),
            name: action.name_of_default(&tool),
        };
        let names = |properties: &[tla_tlc_eval::NamedAction]| {
            properties.iter().map(|(_, name)| name.clone()).collect()
        };
        let info = ModelInfo {
            variables: processed
                .variables
                .iter()
                .map(|&node| tool.name(node).as_str().to_owned())
                .collect(),
            initial_predicates: processed.init_preds.iter().map(action_info).collect(),
            actions: processed.actions.iter().map(action_info).collect(),
            has_next: processed.next_pred.is_some(),
            invariants: names(&processed.invariants),
            initial_properties: names(&processed.implied_inits),
            action_properties: names(&processed.implied_actions),
            temporal_properties: names(&processed.implied_temporals),
            temporal_assumptions: names(&processed.temporals),
            state_constraints: processed.model_constraints.len(),
            action_constraints: processed.action_constraints.len(),
            has_view: processed.view.is_some(),
            has_symmetry: processed.has_symmetry(),
            check_deadlock: processed.config_check_deadlock,
            assumptions: processed.assumptions.len(),
        };
        Ok(Self {
            initial_context: RefCell::new(tool.runtime_snapshot()),
            initial_random: RefCell::new(tla_tlc_value::random::with_random(|r| r.clone())),
            replay_random: Cell::new(true),
            tool,
            processed,
            info,
        })
    }

    pub fn info(&self) -> &ModelInfo {
        &self.info
    }

    /// Collect raw initial states. No partial result is returned after an error.
    pub fn initial_states(&self) -> Result<Vec<State<'_>>> {
        self.begin_initial();
        let mut states = InitVec::default();
        self.tool
            .get_init_states(&mut states)
            .map_err(|e| evaluation_error(Stage::InitialStates, e))?;
        states
            .0
            .into_iter()
            .map(|s| self.complete_state(s, Stage::InitialStates))
            .collect()
    }

    /// Collect raw successors with their faithful split-action IDs. Preserve
    /// duplicates, including a state reached by two different actions.
    pub fn successors<'model>(
        &'model self,
        state: &State<'model>,
    ) -> Result<Vec<Transition<'model>>> {
        self.owns(state)?;
        self.begin_successors(state)?;
        let mut transitions = Vec::new();
        for action in &self.processed.actions {
            let mut states = NextVec::default();
            self.tool
                .get_next_states(action, &state.raw, &mut states)
                .map_err(|e| evaluation_error(Stage::Successors, e))?;
            for mut raw in states.0 {
                raw.set_predecessor(&state.raw);
                raw.set_action(Some(action.id));
                transitions.push(Transition {
                    action: ActionId(action.id),
                    state: self.complete_state(raw, Stage::Successors)?,
                });
            }
        }
        Ok(transitions)
    }

    pub(crate) fn begin_successors(&self, state: &State<'_>) -> Result<()> {
        if !self.replay_random.get() {
            return Ok(());
        }
        let fp = self
            .tool
            .state_fingerprint(&state.raw)
            .map_err(|e| evaluation_error(Stage::Canonicalization, e))?;
        tla_tlc_value::random::with_random(|r| r.set_seed(fp ^ tla_tlc_value::random::get_seed()));
        Ok(())
    }

    /// State-constraint filtering, separate from property checking and generation.
    pub fn state_allowed(&self, state: &State<'_>) -> Result<bool> {
        self.owns(state)?;
        self.tool
            .is_in_model(&state.raw)
            .map_err(|e| evaluation_error(Stage::Constraints, e))
    }

    /// Action-constraint filtering; does not also apply state constraints.
    pub fn step_allowed(&self, from: &State<'_>, to: &State<'_>) -> Result<bool> {
        self.owns(from)?;
        self.owns(to)?;
        self.tool
            .is_in_actions(&from.raw, &to.raw)
            .map_err(|e| evaluation_error(Stage::Constraints, e))
    }

    pub fn take_output(&self) -> Vec<String> {
        self.tool.take_output()
    }

    pub(crate) fn owns(&self, state: &State<'_>) -> Result<()> {
        if std::ptr::eq(self, state.model) {
            Ok(())
        } else {
            Err(ModelError::message(
                Stage::Session,
                "state belongs to another model",
            ))
        }
    }

    pub(crate) fn complete_state(&self, raw: TlcState, stage: Stage) -> Result<State<'_>> {
        if !raw.all_assigned() {
            let missing: Vec<_> = self
                .info
                .variables
                .iter()
                .zip(raw.values())
                .filter(|(_, value)| value.is_none())
                .map(|(name, _)| name.as_str())
                .collect();
            return Err(ModelError {
                stage,
                code: Some(if stage == Stage::InitialStates {
                    mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_INITIAL
                } else {
                    mp::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT
                }),
                message: format!("unassigned variables: {}", missing.join(", ")),
            });
        }
        Ok(State { model: self, raw })
    }
}

impl State<'_> {
    pub fn detached_values(&self) -> Result<Vec<tlaplus_values::Value>> {
        self.raw
            .values()
            .iter()
            .map(|value| crate::values::from_faithful(value.as_ref().expect("complete state")))
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| evaluation_error(Stage::InspectState, e))
    }

    /// TLC's state level, starting at 1; bookkeeping is excluded from equality.
    pub fn level(&self) -> i32 {
        self.raw.level()
    }

    pub fn same_values(&self, other: &State<'_>) -> Result<bool> {
        self.model.owns(other)?;
        self.raw
            .state_equals(&other.raw)
            .map_err(|e| evaluation_error(Stage::InspectState, e))
    }

    pub fn snapshot(&self) -> Result<StateSnapshot> {
        let mut values = BTreeMap::new();
        for (name, value) in self.model.info.variables.iter().zip(self.raw.values()) {
            let value = value.as_ref().expect("complete state");
            value
                .deep_normalize()
                .map_err(|e| evaluation_error(Stage::InspectState, e))?;
            let text = value
                .to_string_checked()
                .map_err(|e| evaluation_error(Stage::InspectState, e))?;
            values.insert(name.clone(), text);
        }
        Ok(StateSnapshot { values })
    }
}

impl Model {
    pub(crate) fn lowered_program(&self) -> tlaplus_ir::Program {
        tlaplus_ir::Program {
            fingerprint_polynomial: Some(tla_tlc_value::fp64::irred_poly() as u64),
            variables: self.info().variables.clone(),
            strings: tla_tlc_value::ustring::snapshot()
                .into_iter()
                .map(|s| (s.as_str().to_owned(), s.tok() as u32))
                .collect(),
            ..tlaplus_ir::Program::default()
        }
    }
    /// Semantic levels of every formula [`crate::Model::lower`] returns, in that order.
    pub fn configured_levels(&self) -> Vec<i32> {
        let level = |id| self.tool.graph().node(id).level.level;
        let mut levels = Vec::new();
        let actions = |levels: &mut Vec<i32>, items: &[tla_tlc_eval::spec::Action]| {
            levels.extend(items.iter().map(|action| level(action.pred.node)));
        };
        let named = |levels: &mut Vec<i32>, items: &[(tla_tlc_eval::spec::Action, String)]| {
            levels.extend(items.iter().map(|(action, _)| level(action.pred.node)));
        };
        actions(&mut levels, &self.processed.init_preds);
        actions(&mut levels, &self.processed.actions);
        named(&mut levels, &self.processed.invariants);
        named(&mut levels, &self.processed.implied_inits);
        named(&mut levels, &self.processed.implied_actions);
        named(&mut levels, &self.processed.implied_temporals);
        named(&mut levels, &self.processed.temporals);
        levels.extend(self.processed.model_constraints.iter().copied().map(level));
        levels.extend(self.processed.action_constraints.iter().copied().map(level));
        if let Some(view) = self.processed.view {
            levels.push(level(view));
        }
        levels
    }

    /// String and model-value ordinals must be the faithful intern table's.
    pub fn check_atom_ordinals(&self, program: &tlaplus_ir::Program) -> Result<()> {
        fn walk(literal: &tlaplus_ir::Literal) -> Result<()> {
            match literal {
                tlaplus_ir::Literal::String { ordinal, text } => atom(text, *ordinal, false),
                tlaplus_ir::Literal::Model { ordinal, text } => atom(text, *ordinal, true),
                tlaplus_ir::Literal::Set(values) | tlaplus_ir::Literal::Tuple(values) => {
                    values.iter().try_for_each(walk)
                }
                tlaplus_ir::Literal::Function(entries) => entries.iter().try_for_each(|(k, v)| {
                    walk(k)?;
                    walk(v)
                }),
                _ => Ok(()),
            }
        }
        fn atom(text: &str, ordinal: u32, model: bool) -> Result<()> {
            let value = crate::values::intern_value(text, model)
                .map_err(|e| ModelError::message(Stage::Load, e))?;
            let expected = match &value {
                tlaplus_values::Value::String(atom) | tlaplus_values::Value::Model(atom) => atom.ordinal,
                _ => return Err(ModelError::message(Stage::Load, "expected an atom")),
            };
            if expected != ordinal {
                return Err(ModelError::message(
                    Stage::Load,
                    format!("atom {text} ordinal {ordinal} != {expected}"),
                ));
            }
            Ok(())
        }
        for node in &program.nodes {
            if let tlaplus_ir::Expr::Literal(literal) = &node.expr {
                walk(literal)?;
            }
        }
        Ok(())
    }
}

impl Model {
    pub(crate) fn seed_context(&self, model: &mut tlaplus_model::BoundModel) -> Result<()> {
        model
            .set_random_seed(
                tla_tlc_value::random::get_seed(),
                self.initial_random.borrow().seed_state(),
            )
            .map_err(|e| ModelError::message(Stage::Load, e))?;
        if let Some(registers) = &self.initial_context.borrow().registers {
            let values = registers
                .first_values()
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        crate::values::from_faithful(&key)?,
                        crate::values::from_faithful(&value)?,
                    ))
                })
                .collect::<std::result::Result<Vec<_>, TlcError>>()
                .map_err(|e| evaluation_error(Stage::Load, e))?;
            model
                .runtime
                .registers
                .seed(values)
                .map_err(|e| ModelError::message(Stage::Load, e))?;
        }
        Ok(())
    }
}

impl Model {
    pub fn random_state(&self) -> u64 {
        tla_tlc_value::random::with_random(|random| random.seed_state())
    }

    pub(crate) fn capture_setup(&self) {
        self.initial_context.replace(self.tool.runtime_snapshot());
        self.initial_random
            .replace(tla_tlc_value::random::with_random(|r| r.clone()));
    }
    pub(crate) fn begin_initial(&self) {
        self.tool
            .restore_runtime(self.initial_context.borrow().clone());
        tla_tlc_value::random::set_current_state(None);
        let saved = self.initial_random.borrow().clone();
        tla_tlc_value::random::with_random(|r| *r = saved);
    }
}

impl Model {
    pub fn first_failed_assumption(&self) -> Option<(usize, Result<bool>)> {
        self.processed
            .assumptions
            .iter()
            .enumerate()
            .find_map(|(index, (id, axiom))| {
                if *axiom {
                    return None;
                }
                match self.tool.is_valid_assumption(*id) {
                    Ok(true) => None,
                    result => Some((
                        index,
                        result.map_err(|error| evaluation_error(Stage::Assumptions, error)),
                    )),
                }
            })
    }
}

pub(crate) fn append_setup_diagnostics(
    error: &mut ModelError,
    records: &[tla_tlc_eval::mp::Record],
) {
    let details = records
        .iter()
        .rev()
        .take(4)
        .map(|r| tla_tlc_eval::mp::get_message(r.code, &r.params))
        .collect::<Vec<_>>();
    if !details.is_empty() {
        error
            .message
            .push_str(&format!(" [{}]", details.join(" | ")));
    }
}

pub(crate) fn evaluation_error(stage: Stage, error: TlcError) -> ModelError {
    let (code, message) = tla_tlc_eval::error_code_and_message(&error);
    ModelError {
        stage,
        code: Some(code),
        message,
    }
}
