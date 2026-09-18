//! Mutable state TLC exposes to specifications: `TLCSet`/`TLCGet` registers,
//! memoization caches and the string intern table, with the snapshots that
//! let worker threads start from one setup.
use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tla_value_expr::Expr;
use tla_values::Value;

/// Why a register or cache operation failed. TLC's own messages are kept
/// verbatim, because specifications observe them through `TLCGet`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A lock on the registers or a cache was poisoned by a panicking thread.
    Poisoned(&'static str),
    /// Setup was used out of order.
    Setup(&'static str),
    /// Workers ended setup with different registers.
    SetupMismatch,
    /// `TLCGet` of a register that was never set.
    Undefined(String),
    /// A runtime key this checker does not provide.
    UnsupportedKey(String),
    /// `TLCSet` outside a running checker.
    Inactive,
    InvalidCacheMetadata,
    /// An error message exactly as TLC reports it.
    Tlc(String),
    Value(tla_values::Error),
    /// The expression a cache was asked to evaluate failed.
    Evaluation(tla_library::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned(what) => write!(f, "{what} poisoned"),
            Self::Setup(what) => f.write_str(what),
            Self::SetupMismatch => f.write_str("setup registers differ between workers"),
            Self::Undefined(name) => write!(f, "TLCGet({name}) was undefined."),
            Self::UnsupportedKey(name) => write!(f, "unsupported TLCGet runtime key: {name}"),
            Self::Inactive => f.write_str("TLCSet requires an active checker"),
            Self::InvalidCacheMetadata => f.write_str("invalid cache metadata"),
            Self::Tlc(message) => f.write_str(message),
            Self::Value(e) => e.fmt(f),
            Self::Evaluation(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<tla_values::Error> for Error {
    fn from(e: tla_values::Error) -> Self {
        Self::Value(e)
    }
}

impl From<Error> for tla_library::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Evaluation(e) => e,
            other => Self(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Key {
    Index(i32),
    Named(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupSnapshot(BTreeMap<Key, Entry>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    key: Value,
    value: Value,
}

#[derive(Debug, Default)]
pub struct Registers {
    initial: SetupSnapshot,
    configured_workers: AtomicUsize,
    workers: Mutex<Option<Vec<BTreeMap<Key, Entry>>>>,
    statistics: Mutex<BTreeMap<String, Value>>,
}

impl Clone for Registers {
    fn clone(&self) -> Self {
        Self {
            initial: self.initial.clone(),
            ..Self::default()
        }
    }
}

fn rendered(value: &Value) -> Result<String, Error> {
    Ok(tla_library::stringify(&Expr::atom(value.clone()))?)
}

fn key(value: &Value, operation: &str) -> Result<Key, Error> {
    match value {
        Value::Int(index) if *index >= 0 => Ok(Key::Index(*index)),
        Value::String(name) if name.text.starts_with("s:") => Ok(Key::Named(name.text.to_string())),
        _ => Err(Error::Tlc(format!(
            "The {}argument of {operation} should be a nonnegative integer, but instead it is:\n{}",
            if operation == "TLCSet" { "first " } else { "" },
            rendered(value)?
        ))),
    }
}

impl Registers {
    pub fn seed(&mut self, values: impl IntoIterator<Item = (Value, Value)>) -> Result<(), Error> {
        self.begin_setup(NonZeroUsize::MIN)?;
        for (key, value) in values {
            self.set(None, &key, value)?;
        }
        self.capture_setup()
    }
    pub fn begin_setup(&mut self, workers: NonZeroUsize) -> Result<(), Error> {
        self.initial = SetupSnapshot::default();
        self.start(workers)
    }
    pub fn capture_setup(&mut self) -> Result<(), Error> {
        let workers = self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?;
        let workers = workers
            .as_ref()
            .ok_or(Error::Setup("register setup has not started"))?;
        let first = workers
            .first()
            .ok_or(Error::Setup("register setup has no workers"))?;
        if workers.iter().any(|worker| worker != first) {
            return Err(Error::SetupMismatch);
        }
        self.initial = SetupSnapshot(first.clone());
        Ok(())
    }
    pub fn setup_snapshot(&self) -> SetupSnapshot {
        self.initial.clone()
    }
    pub fn restore_setup(&mut self, snapshot: SetupSnapshot) {
        self.initial = snapshot;
    }
    pub fn configure(&self, workers: NonZeroUsize) {
        self.configured_workers
            .store(workers.get(), Ordering::Relaxed);
    }
    pub fn reset(&self) -> Result<(), Error> {
        let workers = self.configured_workers.load(Ordering::Relaxed).max(1);
        *self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))? =
            Some(vec![self.initial.0.clone(); workers]);
        Ok(())
    }
    pub fn get_or_default(
        &self,
        worker: Option<usize>,
        argument: &Value,
        default: Value,
    ) -> Result<Value, Error> {
        let key = match argument {
            Value::Int(index) => Key::Index(*index),
            Value::String(name) if name.text.starts_with("s:") => Key::Named(name.text.to_string()),
            _ => return Ok(default),
        };
        let workers = self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?;
        let Some(workers) = workers.as_ref() else {
            return Ok(default);
        };
        let registers = workers
            .get(worker.unwrap_or(0))
            .ok_or(Error::Setup("invalid register worker"))?;
        if let Key::Index(index) = key
            && index < 0
        {
            let length = registers
                .keys()
                .filter_map(|key| match key {
                    Key::Index(index) => Some(*index as usize + 1),
                    _ => None,
                })
                .max()
                .unwrap_or(4)
                .max(4);
            return Err(Error::Tlc(format!(
                "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {length}"
            )));
        }
        Ok(registers
            .get(&key)
            .map(|entry| entry.value.clone())
            .unwrap_or(default))
    }
    pub fn start(&self, workers: NonZeroUsize) -> Result<(), Error> {
        self.configure(workers);
        *self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))? =
            Some(vec![BTreeMap::new(); workers.get()]);
        Ok(())
    }

    /// Publishes checker statistics (`"distinct"`, `"stats"`, ...) for `TLCGet`.
    pub fn publish(&self, statistics: BTreeMap<String, Value>) -> Result<(), Error> {
        self.statistics
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?
            .extend(statistics);
        Ok(())
    }

    pub fn get(&self, worker: Option<usize>, argument: &Value) -> Result<Value, Error> {
        if let Value::String(name) = argument
            && let Some(value) = self
                .statistics
                .lock()
                .map_err(|_| Error::Poisoned("registers"))?
                .get(name.text.as_ref())
        {
            return Ok(value.clone());
        }
        if let Value::String(name) = argument
            && !name.text.starts_with("s:")
        {
            if matches!(name.text.as_ref(), "all" | "all:named") {
                return self.all(name.text.as_ref() == "all:named");
            }
            if name.text.starts_with("-D") {
                return Ok(argument.clone());
            }
            return Err(
                if matches!(
                    name.text.as_ref(),
                    "config"
                        | "spec"
                        | "revision"
                        | "action"
                        | "behavior"
                        | "diameter"
                        | "generated"
                        | "distinct"
                        | "queue"
                        | "stats"
                ) || name.text.starts_with("-D")
                {
                    Error::UnsupportedKey(name.text.to_string())
                } else {
                    Error::Undefined(name.text.to_string())
                },
            );
        }
        let key = key(argument, "TLCGet")?;
        let workers = self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?;
        if let Some(workers) = workers.as_ref() {
            let registers = workers
                .get(worker.unwrap_or(0))
                .ok_or(Error::Setup("invalid register worker"))?;
            if let Some(value) = registers.get(&key) {
                return Ok(value.value.clone());
            }
        }
        let name = match key {
            Key::Index(index) => index.to_string(),
            Key::Named(name) => name,
        };
        Err(Error::Undefined(name))
    }

    fn all(&self, named: bool) -> Result<Value, Error> {
        let workers = self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?;
        let workers = workers
            .as_ref()
            .ok_or_else(|| Error::Undefined(if named { "all:named" } else { "all" }.into()))?;
        let mut entries = Vec::new();
        if let Some(first) = workers.first() {
            for (key, entry) in first {
                if matches!(key, Key::Named(_)) != named {
                    continue;
                }
                let values = workers
                    .iter()
                    .map(|worker| {
                        worker
                            .get(key)
                            .map(|entry| entry.value.clone())
                            .ok_or_else(|| Error::Tlc("java.lang.NullPointerException".into()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                entries.push((entry.key.clone(), Value::tuple(values)));
            }
        }
        Ok(Value::function(entries)?)
    }

    pub fn set(
        &self,
        worker: Option<usize>,
        argument: &Value,
        value: Value,
    ) -> Result<Value, Error> {
        if let Value::String(name) = argument
            && (matches!(name.text.as_ref(), "exit" | "pause") || name.text.starts_with("-D"))
        {
            return Ok(Value::Bool(true));
        }
        let key = key(argument, "TLCSet")?;
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| Error::Poisoned("registers"))?;
        let Some(workers) = workers.as_mut() else {
            return if matches!(key, Key::Named(_)) {
                Ok(Value::Bool(true))
            } else {
                Err(Error::Inactive)
            };
        };
        if let Some(worker) = worker {
            workers
                .get_mut(worker)
                .ok_or(Error::Setup("invalid register worker"))?
                .insert(
                    key,
                    Entry {
                        key: argument.clone(),
                        value,
                    },
                );
        } else {
            for registers in workers {
                registers.insert(
                    key.clone(),
                    Entry {
                        key: argument.clone(),
                        value: value.clone(),
                    },
                );
            }
        }
        Ok(Value::Bool(true))
    }
}

#[derive(Debug, Default)]
pub struct Cache(Mutex<BTreeMap<i32, Vec<(Value, Value)>>>);

impl Cache {
    pub fn evaluate(
        &self,
        metadata: &Value,
        key: Value,
        polynomial: u64,
        evaluate: impl FnOnce() -> Result<Value, tla_library::Error>,
    ) -> Result<Value, Error> {
        let metadata = metadata.sequence()?;
        let [Value::Int(site), seed] = metadata.as_slice() else {
            return Err(Error::InvalidCacheMetadata);
        };
        let site = *site;
        let seed = seed.entries()?;
        let hash = |value: &Value| {
            let fp = value.extend_fingerprint(polynomial, polynomial);
            (fp >> 32) as u32 ^ fp as u32
        };
        {
            let mut entries = self.0.lock().map_err(|_| Error::Poisoned("cache"))?;
            for (stored, value) in entries.entry(site).or_insert_with(|| seed.to_vec()).iter() {
                if hash(stored) == hash(&key) && stored.semantic_eq(&key)? {
                    return Ok(value.clone());
                }
            }
        }
        let value = evaluate().map_err(Error::Evaluation)?;
        self.0
            .lock()
            .map_err(|_| Error::Poisoned("cache"))?
            .entry(site)
            .or_default()
            .push((key, value.clone()));
        Ok(value)
    }
}

#[derive(Debug, Default)]
pub struct Caches {
    setup: BTreeMap<i32, Vec<(Value, Value)>>,
    initial: BTreeMap<i32, Vec<(Value, Value)>>,
    pub active: std::sync::Arc<Cache>,
}

impl Clone for Caches {
    fn clone(&self) -> Self {
        Self {
            setup: self.setup.clone(),
            initial: self.initial.clone(),
            active: std::sync::Arc::new(Cache(Mutex::new(self.initial.clone()))),
        }
    }
}

impl Caches {
    pub fn checkpoint(&self) -> Result<Self, Error> {
        let initial = self
            .active
            .0
            .lock()
            .map_err(|_| Error::Poisoned("cache"))?
            .clone();
        Ok(Self {
            setup: initial.clone(),
            active: std::sync::Arc::new(Cache(Mutex::new(initial.clone()))),
            initial,
        })
    }
    pub fn begin_setup(&self) -> Result<(), Error> {
        *self.active.0.lock().map_err(|_| Error::Poisoned("cache"))? = self.setup.clone();
        Ok(())
    }
    pub fn reset(&self) -> Result<(), Error> {
        *self.active.0.lock().map_err(|_| Error::Poisoned("cache"))? = self.initial.clone();
        Ok(())
    }
    pub fn capture_setup(&mut self) -> Result<(), Error> {
        self.initial = self
            .active
            .0
            .lock()
            .map_err(|_| Error::Poisoned("cache"))?
            .clone();
        Ok(())
    }
}
