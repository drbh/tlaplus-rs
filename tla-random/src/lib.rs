//! Random choice as TLC makes it: `RandomElement`, `RandomSubset` and the
//! randomized set operators, driven by a Java-compatible generator so that
//! seeded runs reproduce TLC's draws.
mod sample;
mod subset;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
};
use tla_values::Error;

#[derive(Debug)]
pub struct Sessions {
    seed: i64,
    setup: Random,
    initial: Random,
    streams: Mutex<HashMap<ThreadId, Arc<Mutex<Random>>>>,
}

impl Clone for Sessions {
    fn clone(&self) -> Self {
        Self {
            setup: self.setup.clone(),
            ..Self::new(self.seed, self.initial.clone())
        }
    }
}

impl Sessions {
    pub fn new(seed: i64, initial: Random) -> Self {
        Self {
            seed,
            setup: initial.clone(),
            initial,
            streams: Mutex::new(HashMap::new()),
        }
    }

    pub fn checkpoint(&self) -> Result<Self, Error> {
        let initial = self
            .current()?
            .lock()
            .map_err(|_| Error::Type("random stream poisoned"))?
            .clone();
        Ok(Self::new(self.seed, initial))
    }

    pub fn begin_setup(&mut self) {
        self.initial = self.setup.clone();
        self.streams = Mutex::new(HashMap::new());
    }

    pub fn capture_setup(&mut self) -> Result<(), Error> {
        self.initial = self
            .current()?
            .lock()
            .map_err(|_| Error::Type("random stream poisoned"))?
            .clone();
        self.streams = Mutex::new(HashMap::new());
        Ok(())
    }

    pub fn current(&self) -> Result<Arc<Mutex<Random>>, Error> {
        Ok(self
            .streams
            .lock()
            .map_err(|_| Error::Type("random sessions poisoned"))?
            .entry(thread::current().id())
            .or_insert_with(|| Arc::new(Mutex::new(self.initial.clone())))
            .clone())
    }

    pub fn begin(&self, fingerprint: Option<u64>) -> Result<(), Error> {
        let random = match fingerprint {
            Some(fp) => Random::new(fp as i64 ^ self.seed),
            None => self.initial.clone(),
        };
        *self
            .current()?
            .lock()
            .map_err(|_| Error::Type("random stream poisoned"))? = random;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Random {
    state: u64,
}

impl Random {
    const MULTIPLIER: u64 = 0x5dee_ce66d;
    const MASK: u64 = (1 << 48) - 1;

    pub fn from_seed_state(state: u64) -> Result<Self, Error> {
        if state & !Self::MASK != 0 {
            return Err(Error::Type("48-bit random seed state"));
        }
        Ok(Self { state })
    }

    pub fn new(seed: i64) -> Self {
        Self {
            state: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
        }
    }
    pub fn state(&self) -> u64 {
        self.state
    }

    fn bits(&mut self, count: u32) -> u32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.state >> (48 - count)) as u32
    }

    pub fn next_double(&mut self) -> f64 {
        let high = u64::from(self.bits(26)) << 27;
        (high + u64::from(self.bits(27))) as f64 / (1u64 << 53) as f64
    }

    pub fn next_bounded(&mut self, bound: i32) -> Result<i32, tla_values::Error> {
        if bound <= 0 {
            return Err(tla_values::Error::Type("positive random bound"));
        }
        let mask = bound - 1;
        loop {
            let bits = self.bits(31) as i32;
            if bound & mask == 0 {
                return Ok(((i64::from(bound) * i64::from(bits)) >> 31) as i32);
            }
            let value = bits % bound;
            if bits.wrapping_sub(value).wrapping_add(mask) >= 0 {
                return Ok(value);
            }
        }
    }

    pub fn next_long(&mut self) -> i64 {
        (i64::from(self.bits(32) as i32) << 32).wrapping_add(i64::from(self.bits(32) as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_replay_setup_and_isolate_worker_streams() {
        let mut initial = Random::new(42);
        initial.next_double();
        let sessions = Sessions::new(42, initial.clone());
        let expected = initial.next_double();
        assert_eq!(
            sessions.current().unwrap().lock().unwrap().next_double(),
            expected
        );
        let cloned = sessions.clone();
        assert_eq!(
            cloned.current().unwrap().lock().unwrap().next_double(),
            expected
        );
        sessions.begin(None).unwrap();
        assert_eq!(
            sessions.current().unwrap().lock().unwrap().next_double(),
            expected
        );
        thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        sessions.begin(Some(0xfeed_beef_7654_3210)).unwrap();
                        let stream = sessions.current().unwrap();
                        let mut stream = stream.lock().unwrap();
                        (stream.next_double(), stream.next_double())
                    })
                })
                .collect();
            let mut expected = Random::new(0xfeed_beef_7654_3210u64 as i64 ^ 42);
            let expected = (expected.next_double(), expected.next_double());
            for worker in workers {
                assert_eq!(worker.join().unwrap(), expected);
            }
        });
        assert!(Random::from_seed_state(1 << 48).is_err());
    }

    #[test]
    fn setup_capture_resets_all_streams_and_preserves_the_setup_origin() {
        let mut sessions = Sessions::new(42, Random::new(42));
        let mut expected = Random::new(42);
        let setup_draw = expected.next_double();
        let initial_draw = expected.next_double();
        for _ in 0..2 {
            sessions.begin_setup();
            assert_eq!(
                sessions.current().unwrap().lock().unwrap().next_double(),
                setup_draw
            );
            sessions.capture_setup().unwrap();
            for run in [&sessions, &sessions.clone(), &sessions] {
                run.begin(None).unwrap();
                assert_eq!(
                    run.current().unwrap().lock().unwrap().next_double(),
                    initial_draw
                );
            }
            thread::scope(|scope| {
                scope
                    .spawn(|| {
                        assert_eq!(
                            sessions.current().unwrap().lock().unwrap().next_double(),
                            initial_draw
                        );
                    })
                    .join()
                    .unwrap();
            });
        }
    }

    #[test]
    fn java_vectors_and_owned_streams() {
        let mut random = Random::new(42);
        assert_eq!(random.bits(32) as i32, -1_170_105_035);
        assert_eq!(random.bits(32), 234_785_527);
        let mut random = Random::new(0);
        assert_eq!(random.next_double(), 0.730967787376657);
        let mut replay = random.clone();
        for _ in 0..100 {
            assert_eq!(random.next_double(), replay.next_double());
        }
        assert_eq!(Random::new(-1), Random::new((1i64 << 48) - 1));
    }
}
