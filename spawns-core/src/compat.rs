use crate::Task;
use linkme::distributed_slice;
use std::sync::OnceLock;

/// Item of [COMPATS] to encapsulate functions to spawn task for async runtime.
#[non_exhaustive]
pub enum Compat {
    /// Named global function to spawn task.
    NamedGlobal { name: &'static str, spawn: fn(Task) },
    /// Global function to spawn task.
    #[doc(hidden)]
    #[deprecated(since = "1.0.3", note = "use NamedGlobal instead")]
    Global(fn(Task)),
    #[allow(clippy::type_complexity)]
    /// Local function to detect async runtimes.
    Local(fn() -> Option<fn(Task)>),
}

/// [DistributedSlice][linkme::DistributedSlice] to collect [Compat]s from
/// [distributed_slice][linkme::distributed_slice].
///
/// Here are examples for `tokio` and `smol`, see [linkme] for details.
///
/// ```rust,no_run
/// use spawns_core as spawns;
///
/// use linkme::distributed_slice;
/// use spawns::{Compat, Task, COMPATS};
///
/// #[distributed_slice(COMPATS)]
/// static TOKIO: Compat = Compat::Local(tokio_local);
///
/// fn tokio_spawn(task: Task) {
///     let Task { future, .. } = task;
///     let handle = tokio::runtime::Handle::current();
///     handle.spawn(Box::into_pin(future));
/// }
///
/// fn tokio_local() -> Option<fn(Task)> {
///     tokio::runtime::Handle::try_current()
///         .ok()
///         .map(|_| tokio_spawn as fn(Task))
/// }
///
/// #[distributed_slice(COMPATS)]
/// static SMOL: Compat = Compat::NamedGlobal {
///     name: "smol",
///     spawn: smol_global,
/// };
///
/// fn smol_global(task: Task) {
///     let Task { future, .. } = task;
///     smol::spawn(Box::into_pin(future)).detach()
/// }
/// ```
#[distributed_slice]
pub static COMPATS: [Compat] = [..];

#[derive(Clone, Copy)]
pub(crate) enum Failure {
    NotFound,
    #[allow(dead_code)]
    MultipleGlobals,
}

fn pick_global(choose: Option<&str>) -> Result<fn(Task), Failure> {
    let mut globals = 0;
    let mut last_named = None;
    let mut last_unnamed = None;
    match COMPATS.iter().find_map(|compat| match compat {
        Compat::Local(_) => None,
        #[allow(deprecated)]
        Compat::Global(global) => {
            globals += 1;
            last_unnamed = Some(global);
            None
        }
        Compat::NamedGlobal { spawn, name } => {
            if choose == Some(name) {
                Some(spawn)
            } else {
                globals += 1;
                last_named = Some(spawn);
                None
            }
        }
    }) {
        Some(spawn) => Ok(*spawn),
        None => {
            #[cfg(feature = "panic-multiple-global-spawners")]
            if globals > 1 {
                return Err(Failure::MultipleGlobals);
            }
            last_named
                .or(last_unnamed)
                .ok_or(Failure::NotFound)
                .copied()
        }
    }
}

fn find_global() -> Result<fn(Task), Failure> {
    static FOUND: OnceLock<Result<fn(Task), Failure>> = OnceLock::new();
    if let Some(found) = FOUND.get() {
        return *found;
    }
    let choose = std::env::var("SPAWNS_GLOBAL_SPAWNER").ok();
    let result = pick_global(choose.as_deref());
    *FOUND.get_or_init(|| result)
}

fn find_local() -> Option<fn(Task)> {
    COMPATS.iter().find_map(|compat| match compat {
        Compat::Local(local) => local(),
        #[allow(deprecated)]
        Compat::Global(_) => None,
        Compat::NamedGlobal { .. } => None,
    })
}

pub(crate) fn find_spawn() -> Option<fn(Task)> {
    match COMPATS.len() {
        0 => return None,
        1 => match COMPATS[0] {
            Compat::NamedGlobal { spawn, .. } => return Some(spawn),
            #[allow(deprecated)]
            Compat::Global(spawn) => return Some(spawn),
            Compat::Local(local) => return local(),
        },
        _ => {}
    }
    match find_local()
        .ok_or(Failure::NotFound)
        .or_else(|_| find_global())
    {
        Ok(spawn) => Some(spawn),
        Err(Failure::NotFound) => None,
        Err(Failure::MultipleGlobals) => panic!("multiple global spawners"),
    }
}
