use crate::{Options, Report, TransitionSystem, search};
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
    T::Key: Send,
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
    // Failed expansions replay lazily on the coordinator, preserving early violations.
    search(
        system,
        options,
        |states| {
            if states.len() < 2 {
                return states
                    .iter()
                    .map(|state| system.successors(state).ok())
                    .collect();
            }
            pool.install(|| {
                states
                    .par_iter()
                    .map(|state| system.successors(state).ok())
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
                    let mut search = search.lock().expect("search lock poisoned");
                    if search.halted {
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                    produced = true;
                    match search.transition(*from, transition) {
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
