//! Executable builder for standalone migration binaries.
//!
//! This module provides functionality to build standalone executables from
//! compiled WebAssembly migration components. The resulting executables embed
//! the Wasm component and can be run without requiring Tern to be installed.
//!
//! # Overview
//!
//! The `ExecutableBuilder` supports two compilation pipelines:
//!
//! ## WASI Pipeline (preferred when available)
//!
//! When embedded WASI components are available, the builder uses:
//!
//! 1. **Data Component Generation**: Creates a WASM component containing SQL data
//! 2. **Component Composition**: Composes guest + data → migration component
//! 3. **Component Composition**: Composes runner + migration → WASI CLI app
//! 4. **AOT Compilation**: Compiles the WASI component to native code
//!
//! This approach requires no external tools at runtime.
//!
//! ## Cargo Pipeline (fallback)
//!
//! When embedded WASI components are not available, the builder falls back to:
//!
//! 1. Creating a temporary Cargo project based on the migration runner template
//! 2. Embedding the Wasm component bytes into the runner
//! 3. Compiling for the target platform using `cargo build`
//! 4. Copying the resulting binary to the output path
//!
//! # Example
//!
//! ```ignore
//! use tern::db::compile::{ExecutableBuilder, Target};
//!
//! let builder = ExecutableBuilder::new();
//! builder.build(
//!     &wasm_component_bytes,
//!     Path::new("./migrations/add_email"),
//!     Target::Native,
//! )?;
//! ```

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::aot::{AotCompiler, AotTarget};
use super::composer::ComponentComposer;
use super::data_component::{DataComponentGenerator, MigrationData};
use super::embedded::{components_available, guest_component, runner_component};
use super::error::CompileError;

// =============================================================================
// Target Platform
// =============================================================================

/// Target platform for executable compilation.
///
/// Specifies the platform and architecture to compile the standalone
/// migration executable for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Native platform (current host).
    #[default]
    Native,
    /// Linux x86_64 with glibc.
    X86_64LinuxGnu,
    /// Linux x86_64 with musl (static linking).
    X86_64LinuxMusl,
    /// macOS x86_64 (Intel).
    X86_64MacOS,
    /// macOS aarch64 (Apple Silicon).
    Aarch64MacOS,
    /// Windows x86_64.
    X86_64Windows,
}

impl Target {
    /// Returns the Rust target triple for this platform.
    ///
    /// Returns `None` for `Target::Native` since it uses the default target.
    pub fn triple(&self) -> Option<&'static str> {
        match self {
            Self::Native => None,
            Self::X86_64LinuxGnu => Some("x86_64-unknown-linux-gnu"),
            Self::X86_64LinuxMusl => Some("x86_64-unknown-linux-musl"),
            Self::X86_64MacOS => Some("x86_64-apple-darwin"),
            Self::Aarch64MacOS => Some("aarch64-apple-darwin"),
            Self::X86_64Windows => Some("x86_64-pc-windows-msvc"),
        }
    }

    /// Returns the executable file extension for this platform.
    pub fn exe_extension(&self) -> &'static str {
        match self {
            Self::X86_64Windows => ".exe",
            _ => "",
        }
    }

    /// Returns whether this target requires cross-compilation tooling.
    pub fn requires_cross_compilation(&self) -> bool {
        !matches!(self, Self::Native)
    }

    /// Parse a target from a string representation.
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" => Some(Self::Native),
            "x86_64-linux-gnu" | "x86_64_linux_gnu" | "linux-gnu" => Some(Self::X86_64LinuxGnu),
            "x86_64-linux-musl" | "x86_64_linux_musl" | "linux-musl" => Some(Self::X86_64LinuxMusl),
            "x86_64-macos" | "x86_64_macos" | "macos-intel" => Some(Self::X86_64MacOS),
            "aarch64-macos" | "aarch64_macos" | "macos-arm" | "macos-silicon" => {
                Some(Self::Aarch64MacOS)
            }
            "x86_64-windows" | "x86_64_windows" | "windows" => Some(Self::X86_64Windows),
            _ => None,
        }
    }

    /// Returns all supported targets.
    pub fn all() -> &'static [Target] {
        &[
            Self::Native,
            Self::X86_64LinuxGnu,
            Self::X86_64LinuxMusl,
            Self::X86_64MacOS,
            Self::Aarch64MacOS,
            Self::X86_64Windows,
        ]
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => write!(f, "native"),
            Self::X86_64LinuxGnu => write!(f, "x86_64-linux-gnu"),
            Self::X86_64LinuxMusl => write!(f, "x86_64-linux-musl"),
            Self::X86_64MacOS => write!(f, "x86_64-macos"),
            Self::Aarch64MacOS => write!(f, "aarch64-macos"),
            Self::X86_64Windows => write!(f, "x86_64-windows"),
        }
    }
}

// =============================================================================
// Executable Builder
// =============================================================================

/// Builder for standalone migration executables.
///
/// Takes a compiled WebAssembly component and produces a platform-native
/// executable that embeds the component and can run migrations independently.
#[derive(Debug, Clone)]
pub struct ExecutableBuilder {
    /// Path to the runner crate template.
    runner_crate_path: PathBuf,
    /// Whether to build in release mode.
    release: bool,
}

impl ExecutableBuilder {
    /// Creates a new executable builder.
    ///
    /// Uses the default runner crate path relative to the tern installation.
    pub fn new() -> Self {
        Self {
            runner_crate_path: Self::default_runner_path(),
            release: true,
        }
    }

    /// Creates a builder with a custom runner crate path.
    pub fn with_runner_path(runner_crate_path: impl Into<PathBuf>) -> Self {
        Self {
            runner_crate_path: runner_crate_path.into(),
            release: true,
        }
    }

    /// Sets whether to build in release mode (default: true).
    pub fn release(mut self, release: bool) -> Self {
        self.release = release;
        self
    }

    /// Returns the default path to the runner crate.
    fn default_runner_path() -> PathBuf {
        // Try to find the runner crate relative to the current executable
        // or fall back to the workspace path
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(parent) = exe_path.parent()
        {
            // Check if we're in a cargo target directory
            let workspace_runner = parent
                .ancestors()
                .find(|p| p.join("Cargo.toml").exists())
                .map(|p| p.join("crates/tern-migration-runner"));

            if let Some(path) = workspace_runner
                && path.exists()
            {
                return path;
            }
        }

        // Fall back to relative path from current directory
        PathBuf::from("crates/tern-migration-runner")
    }

    /// Builds a standalone executable from migration data.
    ///
    /// This method uses the WASI pipeline when embedded components are available,
    /// falling back to the cargo-based approach otherwise.
    ///
    /// # Arguments
    ///
    /// * `migration_data` - The migration data (SQL statements and metadata)
    /// * `output_path` - Where to write the resulting executable
    /// * `target` - The target platform to compile for
    ///
    /// # Errors
    ///
    /// Returns an error if compilation fails.
    pub fn build_from_data(
        &self,
        migration_data: &MigrationData,
        output_path: &Path,
        target: Target,
    ) -> Result<BuildResult, CompileError> {
        // Try WASI pipeline first if components are available
        if components_available() {
            info!("Using WASI pipeline for migration compilation");
            return self.build_wasi_pipeline(migration_data, output_path, target);
        }

        // Fall back to cargo-based approach
        warn!("Embedded WASI components not available, falling back to cargo-based compilation");

        // Generate a minimal component for the cargo approach
        let generator = DataComponentGenerator::new();
        let data_component = generator.generate(migration_data)?;

        self.build(&data_component, output_path, target)
    }

    /// Builds using the WASI pipeline (component composition + AOT).
    ///
    /// This method:
    /// 1. Generates a data component from migration SQL
    /// 2. Composes guest + data → migration component
    /// 3. Composes runner + migration → complete WASI CLI app
    /// 4. AOT compiles to native code
    fn build_wasi_pipeline(
        &self,
        migration_data: &MigrationData,
        output_path: &Path,
        target: Target,
    ) -> Result<BuildResult, CompileError> {
        debug!("Generating data component");
        let generator = DataComponentGenerator::new();
        let data_component = generator.generate(migration_data)?;

        debug!("Loading embedded components");
        let guest_bytes = guest_component()?;
        let runner_bytes = runner_component()?;

        debug!("Composing migration component (guest + data)");
        let composer = ComponentComposer::new()?;
        let migration_component = composer.compose_migration(guest_bytes, &data_component)?;

        debug!("Composing executable (runner + migration)");
        let complete_component = composer.compose_executable(runner_bytes, &migration_component)?;

        debug!("AOT compiling to native code");
        let aot_target = target_to_aot_target(target);
        let aot_compiler = AotCompiler::for_target(aot_target)?;
        let aot_result = aot_compiler.compile(&complete_component)?;

        // Save the compiled result
        debug!("Saving executable to {:?}", output_path);
        aot_result.save_to_file(output_path)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(output_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(output_path, perms);
            }
        }

        Ok(BuildResult {
            output_path: output_path.to_path_buf(),
            target,
            component_size: complete_component.len(),
        })
    }

    /// Builds a standalone executable from a compiled Wasm component.
    ///
    /// This is the legacy cargo-based approach used when WASI components
    /// are not available.
    ///
    /// # Arguments
    ///
    /// * `component_bytes` - The compiled WebAssembly component bytes
    /// * `output_path` - Where to write the resulting executable
    /// * `target` - The target platform to compile for
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The runner crate cannot be found
    /// - The temporary build directory cannot be created
    /// - The cargo build fails
    /// - The output file cannot be written
    pub fn build(
        &self,
        component_bytes: &[u8],
        output_path: &Path,
        target: Target,
    ) -> Result<BuildResult, CompileError> {
        // Verify runner crate exists
        if !self.runner_crate_path.exists() {
            return Err(CompileError::runner_not_found(&self.runner_crate_path));
        }

        // Create temporary build directory
        let temp_dir = tempfile::tempdir().map_err(CompileError::temp_dir)?;
        let build_dir = temp_dir.path();

        // Set up the build project
        self.setup_build_project(build_dir, component_bytes)?;

        // Run cargo build
        let binary_path = self.run_cargo_build(build_dir, target)?;

        // Copy the binary to the output path
        self.copy_output(&binary_path, output_path, target)?;

        Ok(BuildResult {
            output_path: output_path.to_path_buf(),
            target,
            component_size: component_bytes.len(),
        })
    }

    /// Sets up the temporary build project with the embedded component.
    fn setup_build_project(
        &self,
        build_dir: &Path,
        component_bytes: &[u8],
    ) -> Result<(), CompileError> {
        // Copy the runner crate to the build directory
        self.copy_runner_crate(build_dir)?;

        // Write the component bytes to a file that will be included
        let component_path = build_dir.join("migration_component.wasm");
        fs::write(&component_path, component_bytes)
            .map_err(|e| CompileError::io(component_path.display().to_string(), e))?;

        // Create/update the build.rs to embed the component
        self.create_build_script(build_dir, &component_path)?;

        Ok(())
    }

    /// Copies the runner crate to the build directory.
    fn copy_runner_crate(&self, build_dir: &Path) -> Result<(), CompileError> {
        copy_dir_recursive(&self.runner_crate_path, build_dir)
    }

    /// Creates the build script that embeds the component.
    fn create_build_script(
        &self,
        build_dir: &Path,
        component_path: &Path,
    ) -> Result<(), CompileError> {
        let build_rs = format!(
            r#"fn main() {{
    println!("cargo:rerun-if-changed=migration_component.wasm");
    println!("cargo:rustc-env=MIGRATION_COMPONENT_PATH={}");
}}"#,
            component_path.display()
        );

        let build_rs_path = build_dir.join("build.rs");
        fs::write(&build_rs_path, build_rs)
            .map_err(|e| CompileError::io(build_rs_path.display().to_string(), e))?;

        Ok(())
    }

    /// Runs cargo build and returns the path to the built binary.
    fn run_cargo_build(&self, build_dir: &Path, target: Target) -> Result<PathBuf, CompileError> {
        let mut cmd = Command::new("cargo");
        cmd.arg("build");
        cmd.current_dir(build_dir);

        if self.release {
            cmd.arg("--release");
        }

        // Add target if not native
        if let Some(triple) = target.triple() {
            cmd.arg("--target").arg(triple);
        }

        let output = cmd.output().map_err(|e| {
            CompileError::cargo_build(format!("failed to execute cargo: {}", e), None)
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(CompileError::cargo_build(
                format!("build failed with status: {}", output.status),
                Some(stderr),
            ));
        }

        // Determine the path to the built binary
        let profile = if self.release { "release" } else { "debug" };
        let binary_name = format!("tern-migration-runner{}", target.exe_extension());

        let binary_path = if let Some(triple) = target.triple() {
            build_dir
                .join("target")
                .join(triple)
                .join(profile)
                .join(&binary_name)
        } else {
            build_dir.join("target").join(profile).join(&binary_name)
        };

        if !binary_path.exists() {
            return Err(CompileError::cargo_build(
                format!("built binary not found at: {}", binary_path.display()),
                None,
            ));
        }

        Ok(binary_path)
    }

    /// Copies the built binary to the output path.
    fn copy_output(
        &self,
        binary_path: &Path,
        output_path: &Path,
        target: Target,
    ) -> Result<(), CompileError> {
        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;
        }

        // Add extension if needed and not already present
        let final_output = if !output_path
            .to_string_lossy()
            .ends_with(target.exe_extension())
            && !target.exe_extension().is_empty()
        {
            output_path.with_extension(&target.exe_extension()[1..]) // Remove leading dot
        } else {
            output_path.to_path_buf()
        };

        fs::copy(binary_path, &final_output)
            .map_err(|e| CompileError::output_write(final_output.clone(), e))?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&final_output)
                .map_err(|e| CompileError::output_write(final_output.clone(), e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&final_output, perms)
                .map_err(|e| CompileError::output_write(final_output.clone(), e))?;
        }

        Ok(())
    }

    /// Verifies that the required target is installed.
    pub fn verify_target(&self, target: Target) -> Result<(), CompileError> {
        if let Some(triple) = target.triple() {
            let output = Command::new("rustup")
                .args(["target", "list", "--installed"])
                .output()
                .map_err(|e| {
                    CompileError::cargo_build(
                        format!("failed to check installed targets: {}", e),
                        None,
                    )
                })?;

            let installed = String::from_utf8_lossy(&output.stdout);
            if !installed.lines().any(|line| line.trim() == triple) {
                return Err(CompileError::target_not_installed(triple));
            }
        }
        Ok(())
    }
}

impl Default for ExecutableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Build Result
// =============================================================================

/// Result of building a standalone executable.
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Path to the output executable.
    pub output_path: PathBuf,
    /// Target platform that was compiled for.
    pub target: Target,
    /// Size of the embedded Wasm component in bytes.
    pub component_size: usize,
}

impl BuildResult {
    /// Returns the output path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Returns the target platform.
    pub fn target(&self) -> Target {
        self.target
    }

    /// Returns the component size in bytes.
    pub fn component_size(&self) -> usize {
        self.component_size
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Converts an executable Target to an AOT target.
fn target_to_aot_target(target: Target) -> AotTarget {
    match target {
        Target::Native => AotTarget::Native,
        Target::X86_64LinuxGnu | Target::X86_64LinuxMusl => AotTarget::X86_64Linux,
        Target::X86_64MacOS => AotTarget::X86_64MacOS,
        Target::Aarch64MacOS => AotTarget::Aarch64MacOS,
        Target::X86_64Windows => AotTarget::X86_64Windows,
    }
}

/// Recursively copies a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), CompileError> {
    if !dst.exists() {
        fs::create_dir_all(dst).map_err(|e| CompileError::io(dst.display().to_string(), e))?;
    }

    for entry in fs::read_dir(src).map_err(|e| CompileError::io(src.display().to_string(), e))? {
        let entry = entry.map_err(|e| CompileError::io(src.display().to_string(), e))?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            // Skip target directory and git directory
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)
                .map_err(|e| CompileError::io(path.display().to_string(), e))?;
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod target_tests {
        use super::*;

        #[test]
        fn native_has_no_triple() {
            assert_eq!(Target::Native.triple(), None);
        }

        #[test]
        fn linux_gnu_triple() {
            assert_eq!(
                Target::X86_64LinuxGnu.triple(),
                Some("x86_64-unknown-linux-gnu")
            );
        }

        #[test]
        fn linux_musl_triple() {
            assert_eq!(
                Target::X86_64LinuxMusl.triple(),
                Some("x86_64-unknown-linux-musl")
            );
        }

        #[test]
        fn macos_x86_triple() {
            assert_eq!(Target::X86_64MacOS.triple(), Some("x86_64-apple-darwin"));
        }

        #[test]
        fn macos_arm_triple() {
            assert_eq!(Target::Aarch64MacOS.triple(), Some("aarch64-apple-darwin"));
        }

        #[test]
        fn windows_triple() {
            assert_eq!(
                Target::X86_64Windows.triple(),
                Some("x86_64-pc-windows-msvc")
            );
        }

        #[test]
        fn windows_has_exe_extension() {
            assert_eq!(Target::X86_64Windows.exe_extension(), ".exe");
        }

        #[test]
        fn non_windows_has_no_extension() {
            assert_eq!(Target::Native.exe_extension(), "");
            assert_eq!(Target::X86_64LinuxGnu.exe_extension(), "");
            assert_eq!(Target::X86_64MacOS.exe_extension(), "");
        }

        #[test]
        fn native_does_not_require_cross_compilation() {
            assert!(!Target::Native.requires_cross_compilation());
        }

        #[test]
        fn non_native_requires_cross_compilation() {
            assert!(Target::X86_64LinuxGnu.requires_cross_compilation());
            assert!(Target::X86_64LinuxMusl.requires_cross_compilation());
            assert!(Target::X86_64MacOS.requires_cross_compilation());
            assert!(Target::Aarch64MacOS.requires_cross_compilation());
            assert!(Target::X86_64Windows.requires_cross_compilation());
        }

        #[test]
        fn from_str_name_parses_native() {
            assert_eq!(Target::from_str_name("native"), Some(Target::Native));
            assert_eq!(Target::from_str_name("NATIVE"), Some(Target::Native));
        }

        #[test]
        fn from_str_name_parses_linux() {
            assert_eq!(
                Target::from_str_name("x86_64-linux-gnu"),
                Some(Target::X86_64LinuxGnu)
            );
            assert_eq!(
                Target::from_str_name("linux-gnu"),
                Some(Target::X86_64LinuxGnu)
            );
            assert_eq!(
                Target::from_str_name("linux-musl"),
                Some(Target::X86_64LinuxMusl)
            );
        }

        #[test]
        fn from_str_name_parses_macos() {
            assert_eq!(
                Target::from_str_name("x86_64-macos"),
                Some(Target::X86_64MacOS)
            );
            assert_eq!(
                Target::from_str_name("macos-intel"),
                Some(Target::X86_64MacOS)
            );
            assert_eq!(
                Target::from_str_name("aarch64-macos"),
                Some(Target::Aarch64MacOS)
            );
            assert_eq!(
                Target::from_str_name("macos-silicon"),
                Some(Target::Aarch64MacOS)
            );
        }

        #[test]
        fn from_str_name_parses_windows() {
            assert_eq!(
                Target::from_str_name("windows"),
                Some(Target::X86_64Windows)
            );
            assert_eq!(
                Target::from_str_name("x86_64-windows"),
                Some(Target::X86_64Windows)
            );
        }

        #[test]
        fn from_str_name_returns_none_for_invalid() {
            assert_eq!(Target::from_str_name("invalid"), None);
            assert_eq!(Target::from_str_name(""), None);
        }

        #[test]
        fn display_format() {
            assert_eq!(format!("{}", Target::Native), "native");
            assert_eq!(format!("{}", Target::X86_64LinuxGnu), "x86_64-linux-gnu");
            assert_eq!(format!("{}", Target::X86_64LinuxMusl), "x86_64-linux-musl");
            assert_eq!(format!("{}", Target::X86_64MacOS), "x86_64-macos");
            assert_eq!(format!("{}", Target::Aarch64MacOS), "aarch64-macos");
            assert_eq!(format!("{}", Target::X86_64Windows), "x86_64-windows");
        }

        #[test]
        fn all_targets_returns_all_variants() {
            let all = Target::all();
            assert_eq!(all.len(), 6);
            assert!(all.contains(&Target::Native));
            assert!(all.contains(&Target::X86_64LinuxGnu));
            assert!(all.contains(&Target::X86_64LinuxMusl));
            assert!(all.contains(&Target::X86_64MacOS));
            assert!(all.contains(&Target::Aarch64MacOS));
            assert!(all.contains(&Target::X86_64Windows));
        }

        #[test]
        fn default_is_native() {
            assert_eq!(Target::default(), Target::Native);
        }

        #[test]
        fn serialization_roundtrip() {
            for target in Target::all() {
                let json = serde_json::to_string(target).unwrap();
                let parsed: Target = serde_json::from_str(&json).unwrap();
                assert_eq!(*target, parsed);
            }
        }
    }

    mod builder_tests {
        use super::*;

        #[test]
        fn new_creates_builder() {
            let builder = ExecutableBuilder::new();
            assert!(builder.release);
        }

        #[test]
        fn with_runner_path_sets_path() {
            let builder = ExecutableBuilder::with_runner_path("/custom/path");
            assert_eq!(builder.runner_crate_path, PathBuf::from("/custom/path"));
        }

        #[test]
        fn release_mode_can_be_disabled() {
            let builder = ExecutableBuilder::new().release(false);
            assert!(!builder.release);
        }

        #[test]
        fn default_creates_same_as_new() {
            let default = ExecutableBuilder::default();
            let new = ExecutableBuilder::new();
            assert_eq!(default.release, new.release);
        }

        #[test]
        fn build_fails_if_runner_not_found() {
            let builder = ExecutableBuilder::with_runner_path("/nonexistent/path");
            let result = builder.build(&[], Path::new("/tmp/output"), Target::Native);
            assert!(matches!(
                result,
                Err(CompileError::RunnerCrateNotFound { .. })
            ));
        }
    }

    mod build_result_tests {
        use super::*;

        #[test]
        fn accessors_work() {
            let result = BuildResult {
                output_path: PathBuf::from("/output/migration"),
                target: Target::X86_64LinuxMusl,
                component_size: 1024,
            };

            assert_eq!(result.output_path(), Path::new("/output/migration"));
            assert_eq!(result.target(), Target::X86_64LinuxMusl);
            assert_eq!(result.component_size(), 1024);
        }
    }

    mod copy_dir_tests {
        use super::*;
        use tempfile::tempdir;

        #[test]
        fn copy_dir_recursive_copies_files() {
            let src = tempdir().unwrap();
            let dst = tempdir().unwrap();

            // Create a file in source
            let file_path = src.path().join("test.txt");
            fs::write(&file_path, "test content").unwrap();

            // Copy
            copy_dir_recursive(src.path(), dst.path()).unwrap();

            // Verify
            let copied_file = dst.path().join("test.txt");
            assert!(copied_file.exists());
            assert_eq!(fs::read_to_string(copied_file).unwrap(), "test content");
        }

        #[test]
        fn copy_dir_recursive_copies_subdirs() {
            let src = tempdir().unwrap();
            let dst = tempdir().unwrap();

            // Create a subdirectory with a file
            let subdir = src.path().join("subdir");
            fs::create_dir(&subdir).unwrap();
            fs::write(subdir.join("file.txt"), "content").unwrap();

            // Copy
            copy_dir_recursive(src.path(), dst.path()).unwrap();

            // Verify
            let copied_file = dst.path().join("subdir/file.txt");
            assert!(copied_file.exists());
        }

        #[test]
        fn copy_dir_recursive_skips_target_dir() {
            let src = tempdir().unwrap();
            let dst = tempdir().unwrap();

            // Create a target directory that should be skipped
            let target_dir = src.path().join("target");
            fs::create_dir(&target_dir).unwrap();
            fs::write(target_dir.join("build_artifact"), "artifact").unwrap();

            // Also create a regular file
            fs::write(src.path().join("src.rs"), "code").unwrap();

            // Copy
            copy_dir_recursive(src.path(), dst.path()).unwrap();

            // Verify target was skipped
            assert!(!dst.path().join("target").exists());
            // But regular files were copied
            assert!(dst.path().join("src.rs").exists());
        }
    }
}
