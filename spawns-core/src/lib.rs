#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "compat")]
mod compat;
mod spawn;
mod task;

#[cfg(feature = "compat")]
#[cfg_attr(docsrs, doc(cfg(feature = "compat")))]
pub use compat::*;
pub use spawn::*;
pub use task::*;

#[cfg(feature = "compat")]
#[doc(hidden)]
pub mod __compat {
    pub use crate::compat::*;
}
#[doc(hidden)]
pub mod __spawn {
    pub use crate::spawn::*;
}
#[doc(hidden)]
pub mod __task {
    pub use crate::task::*;
}
