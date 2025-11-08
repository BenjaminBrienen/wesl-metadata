#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![deny(missing_docs)]
//! Structured access to the output of `wesl metadata`.
//!
//! ## Examples
//!
//! Get the current package's metadata with all dependency information.
//!
//! ```rust,ignore
//! # use std::path::Path;
//! # use wesl_metadata::MetadataCommand;
//! let _metadata = MetadataCommand::new().exec().unwrap();
//! ```
//!
//! If you have a program that takes `--manifest-path` as an argument, you can forward that
//! to [`MetadataCommand`]:
//!
//! ```rust,ignore
//! # use wesl_metadata::MetadataCommand;
//! # use std::path::Path;
//! let mut args = std::env::args().skip_while(|val| !val.starts_with("--manifest-path"));
//! let mut cmd = MetadataCommand::new();
//! let manifest_path = match args.next() {
//!     Some(ref p) if p == "--manifest-path" => {
//!         cmd.manifest_path(args.next().unwrap());
//!     }
//!     Some(p) => {
//!         cmd.manifest_path(p.trim_start_matches("--manifest-path="));
//!     }
//!     None => {}
//! };
//!
//! let _metadata = cmd.exec().unwrap();
//! ```
//!
//! Pass features flags, e.g. `--all-features`.
//!
//! ```rust,ignore
//! # use std::path::Path;
//! # use wesl_metadata::MetadataCommand;
//! let _metadata = MetadataCommand::new()
//!     .manifest_path("./wesl.toml")
//!     .exec()
//!     .unwrap();
//! ```

use camino::Utf8PathBuf;
#[cfg(feature = "builder")]
use derive_builder::Builder;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::hash::Hash;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::from_utf8;

pub use camino;
pub use semver;
use semver::Version;

pub use dependency::Dependency;
#[cfg(feature = "builder")]
pub use dependency::DependencyBuilder;
pub use errors::{Error, Result};
use serde::{Deserialize, Serialize};

mod dependency;
mod errors;

/// An "opaque" identifier for a package.
///
/// It is possible to inspect the `repr` field, if the need arises, but its
/// precise format is an implementation detail and is subject to change.
///
/// `Metadata` can be indexed by `PackageId`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PackageId {
	/// The underlying string representation of id.
	pub repr: String,
}

impl fmt::Display for PackageId {
	fn fmt(
		&self,
		formatter: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		fmt::Display::fmt(&self.repr, formatter)
	}
}

/// Helpers for default metadata fields
const fn is_null(value: &serde_json::Value) -> bool {
	matches!(value, serde_json::Value::Null)
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// Starting point for metadata returned by `wesl metadata`
pub struct Metadata {
	/// The package manager of this package (for getting dependency packages).
	pub package_manager: PackageManager,

	/// A list of all crates referenced by this crate (and the crate itself)
	pub packages: Vec<Package>,

	/// Dependencies graph
	pub resolve: Option<Resolve>,

	/// Target directory
	pub target_directory: Utf8PathBuf,

	/// The metadata format version
	pub version: usize,

	/// The directory of the root package
	pub root_package_directory: Utf8PathBuf,
}

/// The package manager used for getting dependencies of the WESL package.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum PackageManager {
	/// The package manager is `npm`.
	Npm,
	/// The package manager is `cargo`.
	Cargo,
}

impl Metadata {
	/// Get the root package of this metadata instance.
	#[must_use]
	pub fn root_package(&self) -> Option<&Package> {
		if let Some(resolve) = &self.resolve {
			// if dependencies are resolved, use `wesl`'s answer
			let root = resolve.root.as_ref()?;
			self.packages.iter().find(|pkg| &pkg.id == root)
		} else {
			// if dependencies aren't resolved, check for a root package manually
			let root_manifest_path = self.root_package_directory.join("wesl.toml");
			self.packages
				.iter()
				.find(|pkg| pkg.manifest_path == root_manifest_path)
		}
	}
}

impl<'item> std::ops::Index<&'item PackageId> for Metadata {
	type Output = Package;

	fn index(
		&self,
		index: &'item PackageId,
	) -> &Self::Output {
		self.packages
			.iter()
			.find(|package| package.id == *index)
			.unwrap_or_else(|| panic!("no package with this id: {index:?}"))
	}
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// A dependency graph
pub struct Resolve {
	/// Nodes in a dependencies graph
	pub nodes: Vec<Node>,

	/// The crate for which the metadata was read.
	pub root: Option<PackageId>,
}

impl<'item> std::ops::Index<&'item PackageId> for Resolve {
	type Output = Node;

	fn index(
		&self,
		index: &'item PackageId,
	) -> &Self::Output {
		self.nodes
			.iter()
			.find(|package| package.id == *index)
			.unwrap_or_else(|| panic!("no Node with this id: {index:?}"))
	}
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// A node in a dependencies graph
pub struct Node {
	/// An opaque identifier for a package
	pub id: PackageId,

	/// Dependencies in a structured format.
	///
	/// `renamed_dependencies` handles renamed dependencies whereas `dependencies` does not.
	#[serde(default)]
	pub renamed_dependencies: Vec<NodeDependency>,

	/// List of opaque identifiers for this node's dependencies.
	/// It doesn't support renamed dependencies. See `renamed_dependencies`.
	pub dependencies: Vec<PackageId>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// A dependency in a node
pub struct NodeDependency {
	/// The name of the dependency's library target.
	/// If the crate was renamed, it is the new name.
	pub name: String,

	/// Package ID (opaque unique identifier)
	pub pkg: PackageId,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[non_exhaustive]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
/// One or more crates described by a single `wesl.toml`
///
/// Each [`target`][Package::targets] of a `Package` will be built as a crate.
/// For more information, see <https://doc.rust-lang.org/book/ch07-01-packages-and-crates.html>.
pub struct Package {
	/// The [`name` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	pub name: String,

	/// The [`version` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	pub version: Version,

	/// The [`authors` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	pub authors: Vec<String>,

	/// An opaque identifier for a package
	pub id: PackageId,

	/// The [`description` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[cfg_attr(feature = "builder", builder(default))]
	pub description: Option<String>,

	/// List of dependencies of this particular package
	#[cfg_attr(feature = "builder", builder(default))]
	pub dependencies: Vec<Dependency>,

	/// The [`license` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[cfg_attr(feature = "builder", builder(default))]
	pub license: Option<String>,

	/// The [`license-file` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`.
	/// If the package is using a nonstandard license, this key may be given instead of
	/// `license`, and must point to a file relative to the manifest.
	#[cfg_attr(feature = "builder", builder(default))]
	pub license_file: Option<Utf8PathBuf>,

	/// Path containing the `wesl.toml`
	pub manifest_path: Utf8PathBuf,

	/// The [`categories` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	pub categories: Vec<String>,

	/// The [`keywords` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	pub keywords: Vec<String>,

	/// The [`readme` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	#[cfg_attr(feature = "builder", builder(default))]
	pub readme: Option<Utf8PathBuf>,

	/// The [`repository` URL](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`
	// can't use `url::Url` because that requires a more recent stable compiler
	#[cfg_attr(feature = "builder", builder(default))]
	pub repository: Option<String>,

	/// The [`homepage` URL](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`.
	#[cfg_attr(feature = "builder", builder(default))]
	pub homepage: Option<String>,

	/// The [`documentation` URL](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136) as given in the `wesl.toml`.
	#[cfg_attr(feature = "builder", builder(default))]
	pub documentation: Option<String>,

	/// The WESL edition for the package (either what's given in the [`edition` field](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136)
	/// or defaulting to [`Edition::E2024`]).
	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	pub edition: Edition,

	/// Contents of the free form [`package.metadata` section](https://github.com/wgsl-tooling-wg/wesl-spec/pull/136).
	///
	/// This contents can be serialized to a struct using serde:
	///
	/// ```rust
	/// use serde::Deserialize;
	/// use serde_json::json;
	///
	/// #[derive(Debug, Deserialize)]
	/// struct SomePackageMetadata {
	///     some_value: i32,
	/// }
	///
	/// let value = json!({
	///     "some_value": 42,
	/// });
	///
	/// let package_metadata: SomePackageMetadata = serde_json::from_value(value).unwrap();
	/// assert_eq!(package_metadata.some_value, 42);
	///
	/// ```
	#[serde(default, skip_serializing_if = "is_null")]
	#[cfg_attr(feature = "builder", builder(default))]
	pub metadata: serde_json::Value,
}

#[cfg(feature = "builder")]
impl PackageBuilder {
	/// Construct a new `PackageBuilder` with all required fields.
	pub fn new<
		Namish: Into<String>,
		Versionish: Into<Version>,
		PackageIdish: Into<PackageId>,
		Pathish: Into<Utf8PathBuf>,
	>(
		name: Namish,
		version: Versionish,
		id: PackageIdish,
		path: Pathish,
	) -> Self {
		Self::default()
			.name(name)
			.version(version)
			.id(id)
			.manifest_path(path)
	}
}

impl Package {
	/// Full path to the license file if one is present in the manifest
	#[must_use]
	pub fn license_file(&self) -> Option<Utf8PathBuf> {
		self.license_file.as_ref().map(|file| {
			self.manifest_path
				.parent()
				.unwrap_or(&self.manifest_path)
				.join(file)
		})
	}

	/// Full path to the readme file if one is present in the manifest
	#[must_use]
	pub fn readme(&self) -> Option<Utf8PathBuf> {
		self.readme.as_ref().map(|file| {
			self.manifest_path
				.parent()
				.unwrap_or(&self.manifest_path)
				.join(file)
		})
	}
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "builder", derive(Builder))]
#[cfg_attr(feature = "builder", builder(pattern = "owned", setter(into)))]
#[non_exhaustive]
/// A single target (lib, bin, example, ...) provided by a crate
pub struct Target {
	/// Name as given in the `wesl.toml` or generated from the file name
	pub name: String,

	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	#[serde(rename = "required-features")]
	/// This target is built only if these features are enabled.
	/// It doesn't apply to `lib` targets.
	pub required_features: Vec<String>,

	/// Path to the main source file of the target
	pub src_path: Utf8PathBuf,

	/// Rust edition for this target
	#[serde(default)]
	#[cfg_attr(feature = "builder", builder(default))]
	pub edition: Edition,

	/// Whether or not this target has doc tests enabled, and the target is
	/// compatible with doc testing.
	///
	/// This is always `true` if running with a version of Cargo older than 1.37.
	#[serde(default = "default_true")]
	#[cfg_attr(feature = "builder", builder(default = "true"))]
	pub doctest: bool,

	/// Whether or not this target is tested by default by `cargo test`.
	///
	/// This is always `true` if running with a version of Cargo older than 1.47.
	#[serde(default = "default_true")]
	#[cfg_attr(feature = "builder", builder(default = "true"))]
	pub test: bool,

	/// Whether or not this target is documented by `cargo doc`.
	///
	/// This is always `true` if running with a version of Cargo older than 1.50.
	#[serde(default = "default_true")]
	#[cfg_attr(feature = "builder", builder(default = "true"))]
	pub doc: bool,
}

/// The WESL edition
///
/// As of writing this comment rust editions 2027 and 2030 are not actually a thing yet but are parsed nonetheless for future proofing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(Default)]
pub enum Edition {
	/// WGSL
	#[serde(rename = "WGSL")]
	#[default]
	Wgsl,
	/// WESL
	#[serde(rename = "WESL")]
	WeslUnstable2025,
}

impl Edition {
	/// Return the string representation of the edition
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Wgsl => "WGSL",
			Self::WeslUnstable2025 => "WESL",
		}
	}
}

impl fmt::Display for Edition {
	fn fmt(
		&self,
		formatter: &mut fmt::Formatter<'_>,
	) -> fmt::Result {
		formatter.write_str(self.as_str())
	}
}

const fn default_true() -> bool {
	true
}

/// A builder for configuring `wesl metadata` invocation.
#[derive(Debug, Clone, Default)]
pub struct MetadataCommand {
	/// Path to `wesl` executable. If not set, this will use the
	/// the `$WESL` environment variable, and if that is not set, will
	/// simply be `wesl`.
	wesl_path: Option<PathBuf>,

	/// Path to `wesl.toml`
	manifest_path: Option<PathBuf>,

	/// Current directory of the `wesl metadata` process.
	current_dir: Option<PathBuf>,

	/// Output information only about the root package and don't fetch dependencies.
	no_dependencies: bool,

	/// Arbitrary command line flags to pass to `wesl`. These will be added
	/// to the end of the command line invocation.
	other_options: Vec<String>,

	/// Arbitrary environment variables to set or remove (depending on
	/// [`Option`] value) when running `wesl`. These will be merged into the
	/// calling environment, overriding any which clash.
	env: BTreeMap<OsString, Option<OsString>>,

	/// Show stderr
	verbose: bool,
}

impl MetadataCommand {
	/// Creates a default `wesl metadata` command, which will look for
	/// `wesl.toml` in the ancestors of the current directory.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}
	/// Path to `wesl` executable. If not set, this will use the
	/// the `$WESL` environment variable, and if that is not set, will
	/// simply be `wesl`.
	pub fn wesl_path<Pathish: Into<PathBuf>>(
		&mut self,
		path: Pathish,
	) -> &mut Self {
		self.wesl_path = Some(path.into());
		self
	}
	/// Path to `wesl.toml`
	pub fn manifest_path<Pathish: Into<PathBuf>>(
		&mut self,
		path: Pathish,
	) -> &mut Self {
		self.manifest_path = Some(path.into());
		self
	}
	/// Current directory of the `wesl metadata` process.
	pub fn current_dir<Pathish: Into<PathBuf>>(
		&mut self,
		path: Pathish,
	) -> &mut Self {
		self.current_dir = Some(path.into());
		self
	}
	/// Output information only about the root package and don't fetch dependencies.
	pub const fn no_dependencies(&mut self) -> &mut Self {
		self.no_dependencies = true;
		self
	}

	/// Arbitrary command line flags to pass to `wesl`.
	/// These will be added to the end of the command line invocation.
	pub fn other_options<Options: Into<Vec<String>>>(
		&mut self,
		options: Options,
	) -> &mut Self {
		self.other_options = options.into();
		self
	}

	/// Arbitrary environment variables to set when running `wesl`.
	/// These will be merged into the calling environment, overriding any which clash.
	///
	/// Some examples of when you may want to use this:
	/// 1. Setting cargo config values without needing a .cargo/config.toml file, e.g. to set
	///    `CARGO_NET_GIT_FETCH_WITH_CLI=true`
	/// 2. To specify a custom path to RUSTC if your rust toolchain components aren't laid out in
	///    the way cargo expects by default.
	///
	/// ```no_run
	/// # use wesl_metadata::MetadataCommand;
	/// MetadataCommand::new()
	///     .env("OUT_DIR", "example/value")
	///     // ...
	///     # ;
	/// ```
	pub fn env<K: Into<OsString>, V: Into<OsString>>(
		&mut self,
		key: K,
		val: V,
	) -> &mut Self {
		self.env.insert(key.into(), Some(val.into()));
		self
	}

	/// Arbitrary environment variables to remove when running `cargo`. These will be merged into
	/// the calling environment, overriding any which clash.
	///
	/// Some examples of when you may want to use this:
	/// - Removing inherited environment variables in build scripts that can cause an error
	///   when calling `wesl metadata` (for example, when cross-compiling).
	///
	/// ```no_run
	/// # use wesl_metadata::MetadataCommand;
	/// MetadataCommand::new()
	///     .env_remove("CARGO_ENCODED_RUSTFLAGS")
	///     // ...
	///     # ;
	/// ```
	pub fn env_remove<K: Into<OsString>>(
		&mut self,
		key: K,
	) -> &mut Self {
		self.env.insert(key.into(), None);
		self
	}

	/// Set whether to show stderr
	pub const fn verbose(
		&mut self,
		verbose: bool,
	) -> &mut Self {
		self.verbose = verbose;
		self
	}

	/// Builds a command for `wesl metadata`. This is the first
	/// part of the work of `exec`.
	#[must_use]
	pub fn wesl_command(&self) -> Command {
		let wesl = self
			.wesl_path
			.clone()
			.or_else(|| env::var("WESL").map(PathBuf::from).ok())
			.unwrap_or_else(|| PathBuf::from("wesl"));
		let mut cmd = Command::new(wesl);
		cmd.arg("metadata");

		if self.no_dependencies {
			cmd.arg("--no-dependencies");
		}

		if let Some(path) = self.current_dir.as_ref() {
			cmd.current_dir(path);
		}

		if let Some(manifest_path) = &self.manifest_path {
			cmd.arg("--manifest-path").arg(manifest_path.as_os_str());
		}
		cmd.args(&self.other_options);

		for (key, val) in &self.env {
			match val {
				Some(val) => cmd.env(key, val),
				None => cmd.env_remove(key),
			};
		}

		cmd
	}

	/// Parses `wesl metadata` output. `data` must have been
	/// produced by a command built with `wesl_command`.
	pub fn parse<T: AsRef<str>>(data: T) -> Result<Metadata> {
		let meta = serde_json::from_str(data.as_ref())?;
		Ok(meta)
	}

	/// Runs configured `wesl metadata` and returns parsed `Metadata`.
	pub fn exec(&self) -> Result<Metadata> {
		let mut command = self.wesl_command();
		if self.verbose {
			command.stderr(Stdio::inherit());
		}
		let output = command.output()?;
		if !output.status.success() {
			return Err(Error::WeslMetadata {
				stderr: String::from_utf8(output.stderr)?,
			});
		}
		let stdout = from_utf8(&output.stdout)?
			.lines()
			.find(|line| line.starts_with('{'))
			.ok_or(Error::NoJson)?;
		Self::parse(stdout)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn todo() {}
}
