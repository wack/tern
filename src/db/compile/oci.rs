//! OCI image generation for standalone migration executables.
//!
//! This module provides functionality to package compiled migration executables
//! as OCI (Open Container Initiative) images. The resulting images can be used
//! directly with container runtimes like Docker, Podman, or Kubernetes.
//!
//! # Overview
//!
//! The generated OCI images are "scratch" images containing only:
//! - The statically compiled migration executable
//! - Minimal metadata (config, manifest, index)
//!
//! # Output Format
//!
//! Images are output as tar archives in OCI image layout format:
//!
//! ```text
//! oci-image.tar
//! ├── oci-layout           # OCI layout version marker
//! ├── index.json           # Image index pointing to manifest
//! ├── blobs/
//! │   └── sha256/
//! │       ├── <manifest>   # Image manifest JSON
//! │       ├── <config>     # Image configuration JSON
//! │       └── <layer>      # Gzipped tar of the executable
//! ```
//!
//! # Example
//!
//! ```ignore
//! use tern::db::compile::oci::{OciImageBuilder, OciConfig};
//! use std::path::Path;
//!
//! let builder = OciImageBuilder::new(OciConfig::default());
//! builder.build(
//!     Path::new("./migration"),      // Input executable
//!     Path::new("./migration.tar"),  // Output OCI image tar
//! )?;
//! ```

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tar::Builder as TarBuilder;

use super::Target;
use super::error::CompileError;

// =============================================================================
// OCI Image Configuration
// =============================================================================

/// Configuration for OCI image generation.
#[derive(Debug, Clone)]
pub struct OciConfig {
    /// Name of the executable inside the container.
    pub executable_name: String,
    /// Working directory inside the container.
    pub working_dir: String,
    /// Author/maintainer metadata.
    pub author: Option<String>,
}

impl Default for OciConfig {
    fn default() -> Self {
        Self {
            executable_name: "migration".to_string(),
            working_dir: "/".to_string(),
            author: None,
        }
    }
}

impl OciConfig {
    /// Creates a new OCI configuration with the given executable name.
    pub fn with_executable_name(mut self, name: impl Into<String>) -> Self {
        self.executable_name = name.into();
        self
    }

    /// Sets the author metadata.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

// =============================================================================
// OCI JSON Structures (manually defined for simplicity and compatibility)
// =============================================================================

/// OCI image layout marker.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OciLayout {
    image_layout_version: &'static str,
}

/// OCI descriptor for referencing blobs.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OciDescriptor {
    media_type: &'static str,
    digest: String,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<OciPlatform>,
}

/// OCI platform specification.
#[derive(Debug, Serialize)]
struct OciPlatform {
    architecture: String,
    os: String,
}

/// OCI image index (multi-platform image entry point).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OciIndex {
    schema_version: u32,
    media_type: &'static str,
    manifests: Vec<OciDescriptor>,
}

/// OCI image manifest.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OciManifest {
    schema_version: u32,
    media_type: &'static str,
    config: OciDescriptor,
    layers: Vec<OciDescriptor>,
}

/// OCI image configuration.
#[derive(Debug, Serialize)]
struct OciImageConfig {
    architecture: String,
    os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    config: OciContainerConfig,
    rootfs: OciRootFs,
}

/// Container runtime configuration.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct OciContainerConfig {
    entrypoint: Vec<String>,
    working_dir: String,
}

/// Root filesystem specification.
#[derive(Debug, Serialize)]
struct OciRootFs {
    #[serde(rename = "type")]
    typ: &'static str,
    diff_ids: Vec<String>,
}

// Media type constants
const MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const MEDIA_TYPE_IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const MEDIA_TYPE_IMAGE_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
const MEDIA_TYPE_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

// =============================================================================
// OCI Image Builder
// =============================================================================

/// Builder for OCI container images containing migration executables.
///
/// Creates OCI-compliant images that can be loaded into container runtimes
/// or pushed to container registries.
#[derive(Debug, Clone)]
pub struct OciImageBuilder {
    config: OciConfig,
}

impl OciImageBuilder {
    /// Creates a new OCI image builder with the given configuration.
    pub fn new(config: OciConfig) -> Self {
        Self { config }
    }

    /// Builds an OCI image from a compiled executable.
    ///
    /// # Arguments
    ///
    /// * `executable_path` - Path to the compiled migration executable
    /// * `output_path` - Path for the output tar archive
    /// * `target` - Target platform the executable was compiled for
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The executable cannot be read
    /// - The output file cannot be written
    /// - OCI metadata generation fails
    pub fn build(
        &self,
        executable_path: &Path,
        output_path: &Path,
        target: Target,
    ) -> Result<OciBuildResult, CompileError> {
        // Read the executable
        let executable_bytes = fs::read(executable_path)
            .map_err(|e| CompileError::io(executable_path.display().to_string(), e))?;

        // Determine architecture from target
        let (arch, os) = target_to_oci_platform(target);

        // Build the image
        self.build_from_bytes(&executable_bytes, output_path, &arch, &os)
    }

    /// Builds an OCI image from executable bytes.
    ///
    /// This is the lower-level API that allows building directly from bytes
    /// without reading from a file.
    pub fn build_from_bytes(
        &self,
        executable_bytes: &[u8],
        output_path: &Path,
        arch: &str,
        os: &str,
    ) -> Result<OciBuildResult, CompileError> {
        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;
        }

        // Create the layer tarball (gzipped)
        let layer_data = self.create_layer_tarball(executable_bytes)?;
        let layer_digest = compute_sha256(&layer_data);
        let layer_size = layer_data.len() as u64;

        // Compute the uncompressed layer digest for diff_ids
        let uncompressed_layer = self.create_uncompressed_layer_tarball(executable_bytes)?;
        let uncompressed_digest = compute_sha256(&uncompressed_layer);

        // Create the image configuration
        let config_json =
            self.create_image_config(&uncompressed_digest, arch.to_string(), os.to_string())?;
        let config_bytes = config_json.as_bytes().to_vec();
        let config_digest = compute_sha256(&config_bytes);
        let config_size = config_bytes.len() as u64;

        // Create the manifest
        let manifest_json =
            self.create_manifest(&config_digest, config_size, &layer_digest, layer_size)?;
        let manifest_bytes = manifest_json.as_bytes().to_vec();
        let manifest_digest = compute_sha256(&manifest_bytes);
        let manifest_size = manifest_bytes.len() as u64;

        // Create the index
        let index_json = self.create_index(
            &manifest_digest,
            manifest_size,
            arch.to_string(),
            os.to_string(),
        )?;
        let index_bytes = index_json.as_bytes().to_vec();

        // Create the OCI layout file
        let layout = OciLayout {
            image_layout_version: "1.0.0",
        };
        let layout_json = serde_json::to_string(&layout)
            .map_err(|e| CompileError::oci_generation(format!("layout serialization: {}", e)))?;

        // Build the output tar archive
        let output_file = File::create(output_path)
            .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;

        let mut tar_builder = TarBuilder::new(output_file);

        // Add oci-layout
        add_bytes_to_tar(&mut tar_builder, "oci-layout", layout_json.as_bytes())?;

        // Add index.json
        add_bytes_to_tar(&mut tar_builder, "index.json", &index_bytes)?;

        // Add blobs
        let blobs_prefix = "blobs/sha256";
        add_bytes_to_tar(
            &mut tar_builder,
            &format!("{}/{}", blobs_prefix, manifest_digest),
            &manifest_bytes,
        )?;
        add_bytes_to_tar(
            &mut tar_builder,
            &format!("{}/{}", blobs_prefix, config_digest),
            &config_bytes,
        )?;
        add_bytes_to_tar(
            &mut tar_builder,
            &format!("{}/{}", blobs_prefix, layer_digest),
            &layer_data,
        )?;

        // Finish the tar archive
        tar_builder
            .finish()
            .map_err(|e| CompileError::io(output_path.display().to_string(), e))?;

        Ok(OciBuildResult {
            output_path: output_path.to_path_buf(),
            manifest_digest: format!("sha256:{}", manifest_digest),
            layer_size,
            config_size,
        })
    }

    /// Creates the gzipped layer tarball containing the executable.
    fn create_layer_tarball(&self, executable_bytes: &[u8]) -> Result<Vec<u8>, CompileError> {
        let mut layer_data = Vec::new();
        {
            let gz_encoder = GzEncoder::new(&mut layer_data, Compression::default());
            let mut tar_builder = TarBuilder::new(gz_encoder);

            // Create header for the executable
            let mut header = tar::Header::new_gnu();
            header
                .set_path(&self.config.executable_name)
                .map_err(|e| CompileError::io(self.config.executable_name.clone(), e))?;
            header.set_size(executable_bytes.len() as u64);
            header.set_mode(0o755); // rwxr-xr-x
            header.set_mtime(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            header.set_cksum();

            tar_builder
                .append(&header, executable_bytes)
                .map_err(|e| CompileError::io("layer tarball".to_string(), e))?;

            let gz = tar_builder
                .into_inner()
                .map_err(|e| CompileError::io("layer tarball".to_string(), e))?;
            gz.finish()
                .map_err(|e| CompileError::io("layer compression".to_string(), e))?;
        }
        Ok(layer_data)
    }

    /// Creates the uncompressed layer tarball (for diff_ids computation).
    fn create_uncompressed_layer_tarball(
        &self,
        executable_bytes: &[u8],
    ) -> Result<Vec<u8>, CompileError> {
        let mut layer_data = Vec::new();
        {
            let mut tar_builder = TarBuilder::new(&mut layer_data);

            let mut header = tar::Header::new_gnu();
            header
                .set_path(&self.config.executable_name)
                .map_err(|e| CompileError::io(self.config.executable_name.clone(), e))?;
            header.set_size(executable_bytes.len() as u64);
            header.set_mode(0o755);
            header.set_mtime(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            header.set_cksum();

            tar_builder
                .append(&header, executable_bytes)
                .map_err(|e| CompileError::io("layer tarball".to_string(), e))?;

            tar_builder
                .finish()
                .map_err(|e| CompileError::io("layer tarball".to_string(), e))?;
        }
        Ok(layer_data)
    }

    /// Creates the OCI image configuration JSON.
    fn create_image_config(
        &self,
        layer_diff_id: &str,
        arch: String,
        os: String,
    ) -> Result<String, CompileError> {
        let entrypoint = format!("/{}", self.config.executable_name);

        let config = OciImageConfig {
            architecture: arch,
            os,
            author: self.config.author.clone(),
            config: OciContainerConfig {
                entrypoint: vec![entrypoint],
                working_dir: self.config.working_dir.clone(),
            },
            rootfs: OciRootFs {
                typ: "layers",
                diff_ids: vec![format!("sha256:{}", layer_diff_id)],
            },
        };

        serde_json::to_string(&config)
            .map_err(|e| CompileError::oci_generation(format!("config serialization: {}", e)))
    }

    /// Creates the OCI image manifest JSON.
    fn create_manifest(
        &self,
        config_digest: &str,
        config_size: u64,
        layer_digest: &str,
        layer_size: u64,
    ) -> Result<String, CompileError> {
        let manifest = OciManifest {
            schema_version: 2,
            media_type: MEDIA_TYPE_IMAGE_MANIFEST,
            config: OciDescriptor {
                media_type: MEDIA_TYPE_IMAGE_CONFIG,
                digest: format!("sha256:{}", config_digest),
                size: config_size,
                platform: None,
            },
            layers: vec![OciDescriptor {
                media_type: MEDIA_TYPE_LAYER_GZIP,
                digest: format!("sha256:{}", layer_digest),
                size: layer_size,
                platform: None,
            }],
        };

        serde_json::to_string(&manifest)
            .map_err(|e| CompileError::oci_generation(format!("manifest serialization: {}", e)))
    }

    /// Creates the OCI image index JSON.
    fn create_index(
        &self,
        manifest_digest: &str,
        manifest_size: u64,
        arch: String,
        os: String,
    ) -> Result<String, CompileError> {
        let index = OciIndex {
            schema_version: 2,
            media_type: MEDIA_TYPE_IMAGE_INDEX,
            manifests: vec![OciDescriptor {
                media_type: MEDIA_TYPE_IMAGE_MANIFEST,
                digest: format!("sha256:{}", manifest_digest),
                size: manifest_size,
                platform: Some(OciPlatform {
                    architecture: arch,
                    os,
                }),
            }],
        };

        serde_json::to_string(&index)
            .map_err(|e| CompileError::oci_generation(format!("index serialization: {}", e)))
    }
}

impl Default for OciImageBuilder {
    fn default() -> Self {
        Self::new(OciConfig::default())
    }
}

// =============================================================================
// Build Result
// =============================================================================

/// Result of building an OCI image.
#[derive(Debug, Clone)]
pub struct OciBuildResult {
    /// Path to the output tar archive.
    pub output_path: std::path::PathBuf,
    /// Manifest digest (sha256:...).
    pub manifest_digest: String,
    /// Size of the layer in bytes.
    pub layer_size: u64,
    /// Size of the config in bytes.
    pub config_size: u64,
}

impl OciBuildResult {
    /// Returns the output path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Returns the manifest digest.
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Computes the SHA-256 digest of data and returns the hex string.
fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Converts a compilation target to OCI platform (arch, os).
fn target_to_oci_platform(target: Target) -> (String, String) {
    match target {
        Target::Native => {
            // Detect current platform
            let arch = if cfg!(target_arch = "x86_64") {
                "amd64"
            } else if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "amd64" // Default fallback
            };

            let os = if cfg!(target_os = "linux") {
                "linux"
            } else if cfg!(target_os = "macos") {
                "darwin"
            } else if cfg!(target_os = "windows") {
                "windows"
            } else {
                "linux" // Default fallback
            };

            (arch.to_string(), os.to_string())
        }
        Target::X86_64LinuxGnu | Target::X86_64LinuxMusl => {
            ("amd64".to_string(), "linux".to_string())
        }
        Target::X86_64MacOS => ("amd64".to_string(), "darwin".to_string()),
        Target::Aarch64MacOS => ("arm64".to_string(), "darwin".to_string()),
        Target::X86_64Windows => ("amd64".to_string(), "windows".to_string()),
    }
}

/// Adds bytes to a tar archive with the given path.
fn add_bytes_to_tar<W: Write>(
    tar: &mut TarBuilder<W>,
    path: &str,
    data: &[u8],
) -> Result<(), CompileError> {
    let mut header = tar::Header::new_gnu();
    header
        .set_path(path)
        .map_err(|e| CompileError::io(path.to_string(), e))?;
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    header.set_cksum();

    tar.append(&header, data)
        .map_err(|e| CompileError::io(path.to_string(), e))?;

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn oci_config_default() {
        let config = OciConfig::default();
        assert_eq!(config.executable_name, "migration");
        assert_eq!(config.working_dir, "/");
        assert!(config.author.is_none());
    }

    #[test]
    fn oci_config_builder() {
        let config = OciConfig::default()
            .with_executable_name("my-migration")
            .with_author("Test Author");

        assert_eq!(config.executable_name, "my-migration");
        assert_eq!(config.author, Some("Test Author".to_string()));
    }

    #[test]
    fn target_to_oci_platform_linux() {
        let (arch, os) = target_to_oci_platform(Target::X86_64LinuxMusl);
        assert_eq!(arch, "amd64");
        assert_eq!(os, "linux");
    }

    #[test]
    fn target_to_oci_platform_macos() {
        let (arch, os) = target_to_oci_platform(Target::Aarch64MacOS);
        assert_eq!(arch, "arm64");
        assert_eq!(os, "darwin");
    }

    #[test]
    fn compute_sha256_works() {
        let digest = compute_sha256(b"hello world");
        assert_eq!(
            digest,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn build_oci_image_from_bytes() {
        let temp = tempdir().unwrap();
        let output_path = temp.path().join("test.tar");

        let builder = OciImageBuilder::default();
        let result = builder
            .build_from_bytes(b"#!/bin/sh\necho hello", &output_path, "amd64", "linux")
            .unwrap();

        assert!(result.output_path.exists());
        assert!(result.manifest_digest.starts_with("sha256:"));
        assert!(result.layer_size > 0);
    }

    #[test]
    fn build_oci_image_creates_valid_tar() {
        let temp = tempdir().unwrap();
        let output_path = temp.path().join("test.tar");

        let builder = OciImageBuilder::default();
        builder
            .build_from_bytes(b"test executable content", &output_path, "amd64", "linux")
            .unwrap();

        // Verify the tar can be read
        let file = File::open(&output_path).unwrap();
        let mut archive = tar::Archive::new(file);

        let entries: Vec<_> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path().unwrap().to_string_lossy().into_owned())
            .collect();

        assert!(entries.contains(&"oci-layout".to_string()));
        assert!(entries.contains(&"index.json".to_string()));
        assert!(entries.iter().any(|e| e.starts_with("blobs/sha256/")));
    }
}
