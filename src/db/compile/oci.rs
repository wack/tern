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
use oci_spec::image::{
    Arch, ConfigBuilder, DescriptorBuilder, Digest as OciDigest, ImageConfigurationBuilder,
    ImageIndexBuilder, ImageManifestBuilder, MediaType, OciLayoutBuilder, Os, PlatformBuilder,
    RootFsBuilder, SCHEMA_VERSION, Sha256Digest,
};
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
        self.build_from_bytes(&executable_bytes, output_path, arch, os)
    }

    /// Builds an OCI image from executable bytes.
    ///
    /// This is the lower-level API that allows building directly from bytes
    /// without reading from a file.
    pub fn build_from_bytes(
        &self,
        executable_bytes: &[u8],
        output_path: &Path,
        arch: Arch,
        os: Os,
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
            self.create_image_config(&uncompressed_digest, arch.clone(), os.clone())?;
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
        let index_json = self.create_index(&manifest_digest, manifest_size, arch, os)?;
        let index_bytes = index_json.as_bytes().to_vec();

        // Create the OCI layout file
        let layout = OciLayoutBuilder::default()
            .image_layout_version("1.0.0")
            .build()
            .map_err(|e| CompileError::oci_generation(format!("layout builder: {}", e)))?;
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
        arch: Arch,
        os: Os,
    ) -> Result<String, CompileError> {
        let entrypoint = format!("/{}", self.config.executable_name);

        let container_config = ConfigBuilder::default()
            .entrypoint(vec![entrypoint])
            .working_dir(self.config.working_dir.clone())
            .build()
            .map_err(|e| CompileError::oci_generation(format!("config builder: {}", e)))?;

        let rootfs = RootFsBuilder::default()
            .typ("layers")
            .diff_ids(vec![format!("sha256:{}", layer_diff_id)])
            .build()
            .map_err(|e| CompileError::oci_generation(format!("rootfs builder: {}", e)))?;

        let mut image_config_builder = ImageConfigurationBuilder::default()
            .architecture(arch)
            .os(os)
            .config(container_config)
            .rootfs(rootfs);

        if let Some(ref author) = self.config.author {
            image_config_builder = image_config_builder.author(author.clone());
        }

        let image_config = image_config_builder
            .build()
            .map_err(|e| CompileError::oci_generation(format!("image config: {}", e)))?;

        serde_json::to_string(&image_config)
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
        let config_sha256: Sha256Digest = config_digest
            .parse()
            .map_err(|e| CompileError::oci_generation(format!("config digest parse: {}", e)))?;
        let layer_sha256: Sha256Digest = layer_digest
            .parse()
            .map_err(|e| CompileError::oci_generation(format!("layer digest parse: {}", e)))?;

        let config_descriptor = DescriptorBuilder::default()
            .media_type(MediaType::ImageConfig)
            .digest(OciDigest::from(config_sha256))
            .size(config_size)
            .build()
            .map_err(|e| CompileError::oci_generation(format!("config descriptor: {}", e)))?;

        let layer_descriptor = DescriptorBuilder::default()
            .media_type(MediaType::ImageLayerGzip)
            .digest(OciDigest::from(layer_sha256))
            .size(layer_size)
            .build()
            .map_err(|e| CompileError::oci_generation(format!("layer descriptor: {}", e)))?;

        let manifest = ImageManifestBuilder::default()
            .schema_version(SCHEMA_VERSION)
            .media_type(MediaType::ImageManifest)
            .config(config_descriptor)
            .layers(vec![layer_descriptor])
            .build()
            .map_err(|e| CompileError::oci_generation(format!("manifest: {}", e)))?;

        serde_json::to_string(&manifest)
            .map_err(|e| CompileError::oci_generation(format!("manifest serialization: {}", e)))
    }

    /// Creates the OCI image index JSON.
    fn create_index(
        &self,
        manifest_digest: &str,
        manifest_size: u64,
        arch: Arch,
        os: Os,
    ) -> Result<String, CompileError> {
        let manifest_sha256: Sha256Digest = manifest_digest
            .parse()
            .map_err(|e| CompileError::oci_generation(format!("manifest digest parse: {}", e)))?;

        let platform = PlatformBuilder::default()
            .architecture(arch)
            .os(os)
            .build()
            .map_err(|e| CompileError::oci_generation(format!("platform: {}", e)))?;

        let manifest_descriptor = DescriptorBuilder::default()
            .media_type(MediaType::ImageManifest)
            .digest(OciDigest::from(manifest_sha256))
            .size(manifest_size)
            .platform(platform)
            .build()
            .map_err(|e| CompileError::oci_generation(format!("manifest descriptor: {}", e)))?;

        let index = ImageIndexBuilder::default()
            .schema_version(SCHEMA_VERSION)
            .media_type(MediaType::ImageIndex)
            .manifests(vec![manifest_descriptor])
            .build()
            .map_err(|e| CompileError::oci_generation(format!("index: {}", e)))?;

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
fn target_to_oci_platform(target: Target) -> (Arch, Os) {
    match target {
        Target::Native => {
            // Detect current platform
            let arch = if cfg!(target_arch = "x86_64") {
                Arch::Amd64
            } else if cfg!(target_arch = "aarch64") {
                Arch::ARM64
            } else {
                Arch::Amd64 // Default fallback
            };

            let os = if cfg!(target_os = "linux") {
                Os::Linux
            } else if cfg!(target_os = "macos") {
                Os::Darwin
            } else if cfg!(target_os = "windows") {
                Os::Windows
            } else {
                Os::Linux // Default fallback
            };

            (arch, os)
        }
        Target::X86_64LinuxGnu | Target::X86_64LinuxMusl => (Arch::Amd64, Os::Linux),
        Target::X86_64MacOS => (Arch::Amd64, Os::Darwin),
        Target::Aarch64MacOS => (Arch::ARM64, Os::Darwin),
        Target::X86_64Windows => (Arch::Amd64, Os::Windows),
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
        assert_eq!(arch, Arch::Amd64);
        assert_eq!(os, Os::Linux);
    }

    #[test]
    fn target_to_oci_platform_macos() {
        let (arch, os) = target_to_oci_platform(Target::Aarch64MacOS);
        assert_eq!(arch, Arch::ARM64);
        assert_eq!(os, Os::Darwin);
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
            .build_from_bytes(
                b"#!/bin/sh\necho hello",
                &output_path,
                Arch::Amd64,
                Os::Linux,
            )
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
            .build_from_bytes(
                b"test executable content",
                &output_path,
                Arch::Amd64,
                Os::Linux,
            )
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
