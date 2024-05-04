use crate::Task;
use linkme::distributed_slice;

/// Compat encapsulate functions to find async runtimes to spawn task.
pub enum Compat {
    /// Global function to spawn task.
    ///
    /// [spawn](`crate::spawn()`) will panic if there is no local spawners but multiple global spawners.
    Global(fn(Task)),
    #[allow(clippy::type_complexity)]
    /// Local function to detect async runtimes.
    Local(fn() -> Option<fn(Task)>),
}

/// [DistributedSlice][linkme::DistributedSlice] to collect [Compat]s.
#[distributed_slice]
pub static COMPATS: [Compat] = [..];

pub(crate) fn find_spawn() -> Option<fn(Task)> {
    match COMPATS.len() {
        0 => return None,
        1 => match COMPATS[0] {
            Compat::Global(inject) => return Some(inject),
            Compat::Local(detect) => return detect(),
        },
        _ => {}
    }

    let mut last_global = None;
    let mut globals = 0;
    match COMPATS.iter().find_map(|injection| match injection {
        Compat::Local(local) => local(),
        Compat::Global(global) => {
            globals += 1;
            last_global = Some(global);
            None
        }
    }) {
        Some(spawn) => Some(spawn),
        None => {
            if globals > 1 {
                panic!("multiple global spawners")
            } else {
                last_global.copied()
            }
        }
    }
}
