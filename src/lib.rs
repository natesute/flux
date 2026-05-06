//! `flux` — code-first audiovisual rendering engine.
//!
//! The crate is split into focused modules. Most users will only touch
//! [`project`] (to define a piece) and [`engine`] (to render it).

pub mod audio;
pub mod engine;
pub mod nodes;
pub mod output;
pub mod project;

#[cfg(test)]
pub mod test_utils;

pub use engine::Engine;
pub use project::Project;
