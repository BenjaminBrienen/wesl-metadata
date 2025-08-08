//! This module contains `Dependency` and the types/functions it uses for deserialization.

use camino::Utf8PathBuf;
#[cfg(feature = "builder")]
use derive_builder::Builder;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// A dependency of the main crate
pub struct Dependency {
    /// Name as given in the `wesl.toml`
    pub name: String,

    /// If the dependency is renamed, this is the new name for the dependency
    /// as a string.  None if it is not renamed.
    pub rename: Option<String>,

    /// The file system path for a local path dependency.
    pub path: Option<Utf8PathBuf>,
}
