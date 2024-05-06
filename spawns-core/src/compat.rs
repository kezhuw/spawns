use crate::Task;
use linkme::distributed_slice;
use std::sync::OnceLock;

/// Compat encapsulate functions to find async runtimes to spawn task.
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

/// [DistributedSlice][linkme::DistributedSlice] to collect [Compat]s.
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
