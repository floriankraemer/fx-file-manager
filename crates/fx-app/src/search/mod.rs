//! One recursive search of a tab: its worker, the generation that keeps stale results out, and the result view.

mod slot;
mod view;
mod worker;

pub(crate) use slot::SearchSlot;
pub(crate) use view::SearchEvent;
pub use view::{SearchStatus, SearchView};
pub(crate) use worker::{SearchRequest, SearchRunner};
