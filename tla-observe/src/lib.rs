mod failure;
pub use failure::{Diagnosed, Failure, Phase};
use std::{
    collections::HashMap,
    io::Write,
    num::NonZeroUsize,
    ops::ControlFlow,
    sync::Mutex,
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
use tla_engine::{Transition, TransitionSystem};

#[derive(Default)]
struct Count {
    calls: u64,
    items: u64,
    nanos: u128,
}
struct Event {
    name: &'static str,
    start: Duration,
    duration: Duration,
    thread: usize,
}
#[derive(Default)]
struct Data {
    threads: HashMap<ThreadId, usize>,
    counts: HashMap<(usize, &'static str), Count>,
    events: Vec<Event>,
    dropped: u64,
}
pub struct Timeline {
    start: Instant,
    data: Mutex<Data>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}
impl Timeline {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            data: Mutex::new(Data::default()),
        }
    }
    pub fn measured<'a, T>(&'a self, system: &'a T) -> Measured<'a, T> {
        Measured {
            system,
            timeline: self,
        }
    }
    pub fn span<R>(&self, name: &'static str, operation: impl FnOnce() -> R) -> R {
        let start = Instant::now();
        let result = operation();
        self.record(name, start, 0);
        result
    }
    fn record(&self, name: &'static str, start: Instant, items: usize) {
        let duration = start.elapsed();
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let next = data.threads.len();
        let thread = *data.threads.entry(thread::current().id()).or_insert(next);
        let count = data.counts.entry((thread, name)).or_default();
        count.calls += 1;
        count.items += items as u64;
        count.nanos += duration.as_nanos();
        if data.events.len() < 100_000 {
            data.events.push(Event {
                name,
                start: start.duration_since(self.start),
                duration,
                thread,
            });
        } else {
            data.dropped += 1;
        }
    }
    pub fn write(&self, writer: impl Write) -> Result<(), serde_json::Error> {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let events: Vec<_> = data.events.iter().map(|event| serde_json::json!({"name":event.name,"ph":"X","pid":1,"tid":event.thread,"ts":event.start.as_secs_f64()*1e6,"dur":event.duration.as_secs_f64()*1e6})).collect();
        let mut counts: Vec<_> = data
            .counts
            .iter()
            .map(|(&(thread, name), count)| (thread, name, count))
            .collect();
        counts.sort_by_key(|(thread, name, _)| (*thread, *name));
        let counts: Vec<_> = counts.into_iter().map(|(thread,name,count)| serde_json::json!({"thread":thread,"stage":name,"calls":count.calls,"items":count.items,"nanoseconds":count.nanos})).collect();
        serde_json::to_writer(
            writer,
            &serde_json::json!({"traceEvents":events,"displayTimeUnit":"ms","counters":counts,"droppedEvents":data.dropped}),
        )
    }
}

pub struct Measured<'a, T> {
    system: &'a T,
    timeline: &'a Timeline,
}
impl<T: TransitionSystem> Measured<'_, T> {
    fn measure<R>(&self, name: &'static str, operation: impl FnOnce() -> R) -> R {
        self.timeline.span(name, operation)
    }
}
impl<T: TransitionSystem> TransitionSystem for Measured<'_, T> {
    type State = T::State;
    type Key = T::Key;
    type Error = T::Error;
    fn prepare_parallel(&self, workers: NonZeroUsize) -> Result<(), T::Error> {
        self.system.prepare_parallel(workers)
    }
    fn prefetch_successors(&self) -> bool {
        self.system.prefetch_successors()
    }
    fn parallel_callbacks(&self) -> bool {
        self.system.parallel_callbacks()
    }
    fn initial_states(&self) -> Result<Vec<T::State>, T::Error> {
        self.measure("initial", || self.system.initial_states())
    }
    fn successors(&self, state: &T::State) -> Result<Vec<Transition<T::State>>, T::Error> {
        let start = Instant::now();
        let result = self.system.successors(state);
        self.timeline
            .record("successors", start, result.as_ref().map_or(0, Vec::len));
        result
    }
    fn visit_initial(
        &self,
        out: &mut impl FnMut(T::State) -> Result<ControlFlow<()>, T::Error>,
    ) -> Result<ControlFlow<()>, T::Error> {
        let start = Instant::now();
        let mut count = 0;
        let result = self.system.visit_initial(&mut |state| {
            count += 1;
            out(state)
        });
        self.timeline.record("initial", start, count);
        result
    }
    fn visit_successors(
        &self,
        state: &T::State,
        out: &mut impl FnMut(Transition<T::State>) -> Result<ControlFlow<()>, T::Error>,
    ) -> Result<ControlFlow<()>, T::Error> {
        let start = Instant::now();
        let mut count = 0;
        let result = self.system.visit_successors(state, &mut |transition| {
            count += 1;
            out(transition)
        });
        self.timeline.record("successors", start, count);
        result
    }
    fn key(&self, state: &T::State) -> Result<T::Key, T::Error> {
        self.measure("canonicalize", || self.system.key(state))
    }
    fn same_state(&self, a: &T::State, b: &T::State) -> Result<bool, T::Error> {
        self.system.same_state(a, b)
    }
    fn state_allowed(&self, state: &T::State) -> Result<bool, T::Error> {
        self.measure("state_constraint", || self.system.state_allowed(state))
    }
    fn step_allowed(&self, a: &T::State, b: &T::State) -> Result<bool, T::Error> {
        self.measure("action_constraint", || self.system.step_allowed(a, b))
    }
    fn invariant(&self, state: &T::State) -> Result<Option<String>, T::Error> {
        self.measure("invariant", || self.system.invariant(state))
    }
    fn initial_property(&self, state: &T::State) -> Result<Option<String>, T::Error> {
        self.measure("initial_property", || self.system.initial_property(state))
    }
    fn action_property(&self, a: &T::State, b: &T::State) -> Result<Option<String>, T::Error> {
        self.measure("action_property", || self.system.action_property(a, b))
    }
}
