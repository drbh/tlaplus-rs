use crate::serial::admit;
use crate::{Admitted, Options, Report, TransitionSystem, search};
use rayon::prelude::*;
use std::num::NonZeroUsize;

pub fn explore_parallel<T>(
    system: &T,
    options: &Options,
    workers: NonZeroUsize,
) -> Result<Report<T::State>, T::Error>
where
    T: TransitionSystem + Sync,
    T::State: Send + Sync,
    T::Error: Send,
    T::Key: Send + Sync,
{
    system.prepare_parallel(workers)?;
    if workers.get() > 1 && system.parallel_callbacks() {
        return streaming(system, options, workers);
    }
    if workers.get() == 1 || !system.prefetch_successors() {
        return crate::explore(system, options);
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers.get())
        .build()
        .expect("create evaluator worker pool");
    // Successors and their checks are made on the pool; a failed expansion
    // replays lazily on the coordinator, preserving early violations.
    let expand = |state: &T::State,
                  visited: &std::collections::HashMap<T::Key, usize>|
     -> Option<Vec<Admitted<T::State, T::Key>>> {
        system
            .successors(state)
            .and_then(|transitions| {
                transitions
                    .into_iter()
                    .map(|transition| admit(system, options, Some(visited), state, transition))
                    .collect()
            })
            .ok()
    };
    search(
        system,
        options,
        |states, visited| {
            if states.len() < 2 {
                return states.iter().map(|s| expand(s, visited)).collect();
            }
            pool.install(|| {
                states
                    .par_iter()
                    .map(|s| expand(s, visited))
                    .collect::<Vec<_>>()
            })
        },
        None,
        workers.saturating_mul(NonZeroUsize::new(16).unwrap()),
    )
}

fn streaming<T>(
    system: &T,
    options: &Options,
    workers: NonZeroUsize,
) -> Result<Report<T::State>, T::Error>
where
    T: TransitionSystem + Sync,
    T::State: Send + Sync,
    T::Error: Send,
    T::Key: Send,
{
    let search = std::sync::Mutex::new(crate::serial::Search::new(system, options)?);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers.get())
        .build()
        .expect("create evaluator worker pool");
    loop {
        let frontier = {
            let mut search = search.lock().expect("search lock poisoned");
            if search.halted || search.next.is_empty() {
                break;
            }
            let frontier = std::mem::take(&mut search.next);
            search.report.statistics.peak_frontier =
                search.report.statistics.peak_frontier.max(frontier.len());
            frontier
                .into_iter()
                .map(|from| (from, search.report.nodes[from].state.clone()))
                .collect::<Vec<_>>()
        };
        pool.install(|| {
            frontier.par_iter().for_each(|(from, current)| {
                if search.lock().expect("search lock poisoned").halted {
                    return;
                }
                let mut produced = false;
                let result = system.visit_successors(current, &mut |transition| {
                    // `visited` lives under the lock, so the invariant and
                    // property checks stay with the coordinator here; only
                    // constraints and keys are made on the worker.
                    let admitted = admit(system, options, None, current, transition)?;
                    let mut search = search.lock().expect("search lock poisoned");
                    if search.halted {
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                    produced = true;
                    match search.transition(*from, admitted) {
                        Ok(flow) => Ok(flow),
                        Err(error) => {
                            search.fail(error);
                            Ok(std::ops::ControlFlow::Break(()))
                        }
                    }
                });
                let mut search = search.lock().expect("search lock poisoned");
                if let Err(error) = result {
                    search.fail(error);
                }
                search.expanded(*from, produced);
            })
        });
    }
    search.into_inner().expect("search lock poisoned").finish()
}
