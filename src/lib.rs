#[cfg(test)]
pub mod appearance;
pub mod cli;
pub mod closing;
pub mod commands;
pub mod comparison;
pub mod directory;
pub mod document;
pub mod editor;
pub mod fileops;
pub mod io;
pub mod lifecycle;
#[cfg(test)]
pub mod markdown;
pub mod metrics;
#[cfg(test)]
pub mod navigation;
pub mod observe;
pub mod paths;
pub mod picker_state;
pub mod reservations;
pub mod search;
pub mod storage;
pub mod syntax;
pub mod wake;

#[cfg(test)]
pub use paths::name;
