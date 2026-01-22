//! OCI image generation for migration executables.
//!
//! This module provides functionality to package standalone migration executables
//! as OCI (Open Container Initiative) images. The generated images are minimal
//! "scratch" images containing only the migration binary.
//!
//! # Overview
//!
//! The `OciImageBuilder` takes a compiled migration executable and produces an
//! OCI image as a gzipped tar archive. The image is built "from scratch" with
//! no base image, containing only:
//!
//! - The migration executable as `/migration`
//! - Minimal OCI metadata (config, manifest, index)
//!
//! # Usage
//!
//! ```ignore
//! use tern::db::compile::{OciImageBuilder, Target};
//! use std::path::Path;
//!
//! let builder = OciImageBuilder::new();
//! let result = builder.build(
//!     &executable_bytes,
//!     Path::new("./migrations/add_email.tar.gz"),
//!     Target::X86_64LinuxMusl,
//! )?;
//! ```
//!
//! # OCI Image Structure
//!
//! The generated tar archive follows the OCI Image Layout Specification:
//!
//! ```text
//! oci-layout              # {"imageLayoutVersion": "1.0.0"}
//! index.json              # Points to the manifest
//! blobs/
//!   sha256/
//!     <config-digest>     # Image configuration JSON
//!     <layer-digest>      # Gzipped tar of the rootfs
//!     <manifest-digest>   # Image manifest JSON
//! ```

use std::fs::File;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tar::{Builder as TarBuilder, Header};
use tracing::{debug, info};

use super::error::CompileError;
use super::executable::Target;

// =============================================================================
// Constants
// =============================================================================

/// The path where the migration binary is placed inside the container.
const MIGRATION_BINARY_PATH: &str = "migration";

/// OCI image layout version.
const OCI_LAYOUT_VERSION: &str = "1.0.0";

/// OCI schema version.
const SCHEMA_VERSION: u32 = 2;

/// Media type for OCI image config.
const MEDIA_TYPE_IMAGE_CONFIG: &str = "application/vnd.oci.image.config.v1+json";

/// Media type for OCI image manifest.
const MEDIA_TYPE_IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

/// Media type for OCI image index.
const MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";

/// OCI media type for gzipped tar layers.
const MEDIA_TYPE_LAYER: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

// =============================================================================
// OCI JSON Structures
// =============================================================================

/// OCI image layout marker file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OciLayout {
    image_layout_version: &'static str,
}

impl Default for OciLayout {
    fn default() -> Self {
        Self {
            image_layout_version: OCI_LAYOUT_VERSION,
        }
    }
}

/// OCI content descriptor.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    media_type: &'static str,
    digest: String,
    size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<Platform>,
}

/// OCI platform specification.
#[derive(Debug, Serialize)]
struct Platform {
    architecture: &'static str,
    os: &'static str,
}

/// OCI image index (index.json).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageIndex {
    schema_version: u32,
    media_type: &'static str,
    manifests: Vec<Descriptor>,
}

/// OCI image manifest.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageManifest {
    schema_version: u32,
    media_type: &'static str,
    config: Descriptor,
    layers: Vec<Descriptor>,
}

/// OCI image configuration.
#[derive(Debug, Serialize)]
struct ImageConfig {
    architecture: &'static str,
    os: &'static str,
    config: ContainerConfig,
    rootfs: RootFs,
}

/// Container runtime configuration.
#[derive(Debug, Serialize)]
struct ContainerConfig {
    #[serde(rename = "Entrypoint")]
    entrypoint: Vec<String>,
    #[serde(rename = "Env")]
    env: Vec<String>,
}

/// Root filesystem configuration.
#[derive(Debug, Serialize)]
struct RootFs {
    #[serde(rename = "type")]
    typ: &'static str,
    diff_ids: Vec<String>,
}

// =============================================================================
// OCI Image Builder
// =============================================================================

/// Builder for OCI images containing migration executables.
///
/// Creates minimal "scratch" OCI images with just the migration binary.
/// The resulting images can be pushed to any OCI-compatible registry
/// or loaded directly into container runtimes.
#[derive(Debug, Clone, Default)]
pub struct OciImageBuilder {
    /// Reserved for future configuration options.
    _private: (),
}

impl OciImageBuilder {
    /// Creates a new OCI image builder.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Builds an OCI image from an executable.
    ///
    /// # Arguments
    ///
    /// * `executable_bytes` - The compiled migration executable bytes
    /// * `output_path` - Where to write the resulting tar.gz archive
    /// * `target` - The target platform the executable was compiled for
    ///
    /// # Returns
    ///
    /// Returns an `OciBuildResult` with information about the generated image.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The output file cannot be created
    /// - Writing to the archive fails
    pub fn build(
        &self,
        executable_bytes: &[u8],
        output_path: &Path,
        target: Target,
    ) -> Result<OciBuildResult, CompileError> {
        info!("Building OCI image for target {:?}", target);

        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;
        }

        // Create the output file
        let file = File::create(output_path)
            .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;

        // Create a gzip encoder wrapping the file
        let gz_encoder = GzEncoder::new(file, Compression::default());

        // Create tar builder
        let mut tar = TarBuilder::new(gz_encoder);

        // Step 1: Create the rootfs layer (gzipped tar containing the binary)
        debug!("Creating rootfs layer");
        let (layer_bytes, layer_digest, layer_diff_id) =
            self.create_layer(executable_bytes, target)?;
        let layer_size = layer_bytes.len() as u64;

        // Step 2: Create image configuration
        debug!("Creating image configuration");
        let (config_bytes, config_digest) = self.create_config(target, &layer_diff_id)?;
        let config_size = config_bytes.len() as u64;

        // Step 3: Create image manifest
        debug!("Creating image manifest");
        let (manifest_bytes, manifest_digest) =
            self.create_manifest(&config_digest, config_size, &layer_digest, layer_size)?;
        let manifest_size = manifest_bytes.len() as u64;

        // Step 4: Create index.json
        debug!("Creating image index");
        let index_bytes = self.create_index(&manifest_digest, manifest_size, target)?;

        // Step 5: Write oci-layout file
        debug!("Writing OCI layout");
        let layout = OciLayout::default();
        let layout_bytes =
            serde_json::to_vec_pretty(&layout).map_err(|e| CompileError::Serialization {
                context: "OCI layout".to_string(),
                message: e.to_string(),
            })?;
        self.write_tar_entry(&mut tar, "oci-layout", &layout_bytes)?;

        // Step 6: Write index.json
        self.write_tar_entry(&mut tar, "index.json", &index_bytes)?;

        // Step 7: Write blobs
        self.write_tar_entry(
            &mut tar,
            &format!("blobs/sha256/{}", &config_digest[7..]),
            &config_bytes,
        )?;
        self.write_tar_entry(
            &mut tar,
            &format!("blobs/sha256/{}", &layer_digest[7..]),
            &layer_bytes,
        )?;
        self.write_tar_entry(
            &mut tar,
            &format!("blobs/sha256/{}", &manifest_digest[7..]),
            &manifest_bytes,
        )?;

        // Finish the tar archive
        let gz_encoder = tar
            .into_inner()
            .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;
        gz_encoder
            .finish()
            .map_err(|e| CompileError::output_write(output_path.to_path_buf(), e))?;

        info!("OCI image written to {:?}", output_path);

        Ok(OciBuildResult {
            output_path: output_path.to_path_buf(),
            target,
            manifest_digest,
            layer_size,
            config_size,
        })
    }

    /// Creates the rootfs layer containing the migration binary.
    ///
    /// Returns (gzipped_tar_bytes, compressed_digest, uncompressed_diff_id).
    fn create_layer(
        &self,
        executable_bytes: &[u8],
        target: Target,
    ) -> Result<(Vec<u8>, String, String), CompileError> {
        // Create an uncompressed tar first to compute the diff_id
        let mut uncompressed_tar = Vec::new();
        {
            let mut tar = TarBuilder::new(&mut uncompressed_tar);

            // Create header for the binary
            let mut header = Header::new_gnu();
            header.set_size(executable_bytes.len() as u64);
            header.set_mode(0o755); // Executable
            header.set_cksum();

            // Determine the binary name
            let binary_name = format!("{}{}", MIGRATION_BINARY_PATH, target.exe_extension());

            tar.append_data(&mut header, &binary_name, executable_bytes)
                .map_err(|e| CompileError::Io {
                    path: "layer tar".to_string(),
                    source: e,
                })?;

            tar.into_inner().map_err(|e| CompileError::Io {
                path: "layer tar".to_string(),
                source: e,
            })?;
        }

        // Compute diff_id from uncompressed tar
        let diff_id = format!("sha256:{}", hex::encode(Sha256::digest(&uncompressed_tar)));

        // Now compress the tar
        let mut compressed_tar = Vec::new();
        {
            let mut gz = GzEncoder::new(&mut compressed_tar, Compression::default());
            gz.write_all(&uncompressed_tar)
                .map_err(|e| CompileError::Io {
                    path: "layer gzip".to_string(),
                    source: e,
                })?;
            gz.finish().map_err(|e| CompileError::Io {
                path: "layer gzip".to_string(),
                source: e,
            })?;
        }

        // Compute digest of compressed layer
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&compressed_tar)));

        Ok((compressed_tar, digest, diff_id))
    }

    /// Creates the image configuration JSON.
    ///
    /// Returns (config_bytes, digest).
    fn create_config(
        &self,
        target: Target,
        layer_diff_id: &str,
    ) -> Result<(Vec<u8>, String), CompileError> {
        let (os, arch) = target_to_os_arch(target);

        let binary_path = format!("/{}{}", MIGRATION_BINARY_PATH, target.exe_extension());

        let config = ImageConfig {
            architecture: arch,
            os,
            config: ContainerConfig {
                entrypoint: vec![binary_path],
                env: vec!["PATH=/".to_string()],
            },
            rootfs: RootFs {
                typ: "layers",
                diff_ids: vec![layer_diff_id.to_string()],
            },
        };

        let config_bytes =
            serde_json::to_vec_pretty(&config).map_err(|e| CompileError::Serialization {
                context: "OCI config JSON".to_string(),
                message: e.to_string(),
            })?;

        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&config_bytes)));

        Ok((config_bytes, digest))
    }

    /// Creates the image manifest JSON.
    ///
    /// Returns (manifest_bytes, digest).
    fn create_manifest(
        &self,
        config_digest: &str,
        config_size: u64,
        layer_digest: &str,
        layer_size: u64,
    ) -> Result<(Vec<u8>, String), CompileError> {
        let manifest = ImageManifest {
            schema_version: SCHEMA_VERSION,
            media_type: MEDIA_TYPE_IMAGE_MANIFEST,
            config: Descriptor {
                media_type: MEDIA_TYPE_IMAGE_CONFIG,
                digest: config_digest.to_string(),
                size: config_size,
                platform: None,
            },
            layers: vec![Descriptor {
                media_type: MEDIA_TYPE_LAYER,
                digest: layer_digest.to_string(),
                size: layer_size,
                platform: None,
            }],
        };

        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|e| CompileError::Serialization {
                context: "manifest JSON".to_string(),
                message: e.to_string(),
            })?;

        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&manifest_bytes)));

        Ok((manifest_bytes, digest))
    }

    /// Creates the index.json file.
    fn create_index(
        &self,
        manifest_digest: &str,
        manifest_size: u64,
        target: Target,
    ) -> Result<Vec<u8>, CompileError> {
        let (os, arch) = target_to_os_arch(target);

        let index = ImageIndex {
            schema_version: SCHEMA_VERSION,
            media_type: MEDIA_TYPE_IMAGE_INDEX,
            manifests: vec![Descriptor {
                media_type: MEDIA_TYPE_IMAGE_MANIFEST,
                digest: manifest_digest.to_string(),
                size: manifest_size,
                platform: Some(Platform {
                    architecture: arch,
                    os,
                }),
            }],
        };

        serde_json::to_vec_pretty(&index).map_err(|e| CompileError::Serialization {
            context: "index JSON".to_string(),
            message: e.to_string(),
        })
    }

    /// Writes an entry to the tar archive.
    fn write_tar_entry<W: Write>(
        &self,
        tar: &mut TarBuilder<W>,
        path: &str,
        data: &[u8],
    ) -> Result<(), CompileError> {
        let mut header = Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();

        tar.append_data(&mut header, path, data)
            .map_err(|e| CompileError::Io {
                path: path.to_string(),
                source: e,
            })
    }
}

// =============================================================================
// Build Result
// =============================================================================

/// Result of building an OCI image.
#[derive(Debug, Clone)]
pub struct OciBuildResult {
    /// Path to the output tar.gz archive.
    pub output_path: std::path::PathBuf,
    /// Target platform the image was built for.
    pub target: Target,
    /// Digest of the image manifest.
    pub manifest_digest: String,
    /// Size of the rootfs layer in bytes.
    pub layer_size: u64,
    /// Size of the config in bytes.
    pub config_size: u64,
}

impl OciBuildResult {
    /// Returns the output path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Returns the target platform.
    pub fn target(&self) -> Target {
        self.target
    }

    /// Returns the manifest digest.
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Converts a compilation target to OCI os/architecture strings.
fn target_to_os_arch(target: Target) -> (&'static str, &'static str) {
    match target {
        Target::Native => {
            // Detect native platform
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                ("linux", "amd64")
            }
            #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
            {
                ("linux", "arm64")
            }
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            {
                ("darwin", "amd64")
            }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                ("darwin", "arm64")
            }
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            {
                ("windows", "amd64")
            }
            #[cfg(not(any(
                all(target_os = "linux", target_arch = "x86_64"),
                all(target_os = "linux", target_arch = "aarch64"),
                all(target_os = "macos", target_arch = "x86_64"),
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "windows", target_arch = "x86_64"),
            )))]
            {
                ("linux", "amd64") // Default fallback
            }
        }
        Target::X86_64LinuxGnu | Target::X86_64LinuxMusl => ("linux", "amd64"),
        Target::X86_64MacOS => ("darwin", "amd64"),
        Target::Aarch64MacOS => ("darwin", "arm64"),
        Target::X86_64Windows => ("windows", "amd64"),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    mod oci_builder_tests {
        use super::*;

        #[test]
        fn new_creates_builder() {
            let _builder = OciImageBuilder::new();
        }

        #[test]
        fn default_creates_builder() {
            let _builder = OciImageBuilder::default();
        }

        #[test]
        fn build_creates_valid_tar_gz() {
            let builder = OciImageBuilder::new();
            let temp = tempdir().unwrap();
            let output_path = temp.path().join("test.tar.gz");

            // Create a minimal "executable" (just some bytes)
            let executable = b"#!/bin/sh\necho hello\n";

            let result = builder
                .build(executable, &output_path, Target::X86_64LinuxMusl)
                .unwrap();

            // Verify the file was created
            assert!(output_path.exists());
            assert!(result.layer_size > 0);
            assert!(result.manifest_digest.starts_with("sha256:"));
        }

        #[test]
        fn build_creates_correct_structure() {
            let builder = OciImageBuilder::new();
            let temp = tempdir().unwrap();
            let output_path = temp.path().join("test.tar.gz");

            let executable = b"test binary content";
            builder
                .build(executable, &output_path, Target::X86_64LinuxMusl)
                .unwrap();

            // Open and verify the tar.gz contents
            let file = File::open(&output_path).unwrap();
            let gz = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(gz);

            let mut found_oci_layout = false;
            let mut found_index = false;
            let mut found_blobs = 0;

            for entry in archive.entries().unwrap() {
                let entry = entry.unwrap();
                let path = entry.path().unwrap();
                let path_str = path.to_string_lossy();

                if path_str == "oci-layout" {
                    found_oci_layout = true;
                } else if path_str == "index.json" {
                    found_index = true;
                } else if path_str.starts_with("blobs/sha256/") {
                    found_blobs += 1;
                }
            }

            assert!(found_oci_layout, "oci-layout file not found");
            assert!(found_index, "index.json file not found");
            assert_eq!(found_blobs, 3, "Expected 3 blobs (config, layer, manifest)");
        }

        #[test]
        fn layer_contains_executable() {
            let builder = OciImageBuilder::new();
            let executable = b"test binary";

            let (layer_bytes, digest, diff_id) = builder
                .create_layer(executable, Target::X86_64LinuxMusl)
                .unwrap();

            assert!(!layer_bytes.is_empty());
            assert!(digest.starts_with("sha256:"));
            assert!(diff_id.starts_with("sha256:"));

            // Decompress and check contents
            let gz = flate2::read::GzDecoder::new(&layer_bytes[..]);
            let mut archive = tar::Archive::new(gz);

            let mut found_binary = false;
            for entry in archive.entries().unwrap() {
                let entry = entry.unwrap();
                let path = entry.path().unwrap();
                if path.to_string_lossy() == "migration" {
                    found_binary = true;
                }
            }
            assert!(found_binary, "migration binary not found in layer");
        }
    }

    mod target_conversion_tests {
        use super::*;

        #[test]
        fn linux_targets_have_correct_os_arch() {
            assert_eq!(
                target_to_os_arch(Target::X86_64LinuxGnu),
                ("linux", "amd64")
            );
            assert_eq!(
                target_to_os_arch(Target::X86_64LinuxMusl),
                ("linux", "amd64")
            );
        }

        #[test]
        fn macos_targets_have_correct_os_arch() {
            assert_eq!(target_to_os_arch(Target::X86_64MacOS), ("darwin", "amd64"));
            assert_eq!(target_to_os_arch(Target::Aarch64MacOS), ("darwin", "arm64"));
        }

        #[test]
        fn windows_target_has_correct_os_arch() {
            assert_eq!(
                target_to_os_arch(Target::X86_64Windows),
                ("windows", "amd64")
            );
        }
    }

    mod build_result_tests {
        use super::*;

        #[test]
        fn accessors_work() {
            let result = OciBuildResult {
                output_path: std::path::PathBuf::from("/output/image.tar.gz"),
                target: Target::X86_64LinuxMusl,
                manifest_digest: "sha256:abc123".to_string(),
                layer_size: 1024,
                config_size: 256,
            };

            assert_eq!(result.output_path(), Path::new("/output/image.tar.gz"));
            assert_eq!(result.target(), Target::X86_64LinuxMusl);
            assert_eq!(result.manifest_digest(), "sha256:abc123");
        }
    }
}
