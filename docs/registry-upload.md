# OCI Registry Upload Support

> **Status**: Future Enhancement
> **Priority**: Medium
> **Complexity**: Moderate

## Overview

This document describes a planned enhancement to allow Tern to push OCI images directly to container registries. Currently, Tern can generate OCI images as tar archives that must be manually loaded into a container runtime or pushed to a registry using external tools. Direct registry upload would streamline the workflow for users deploying migrations to Kubernetes environments.

## Current State

The `tern build` command with `--package-format oci` produces a tar archive in OCI image layout format:

```bash
tern build \
  --database-url "$DATABASE_URL" \
  --description "Add users table" \
  --output ./migration.tar \
  --package-format oci
```

Users must then use external tools to push to a registry:

```bash
# Using skopeo
skopeo copy oci-archive:migration.tar docker://registry.example.com/migrations/add-users:v1

# Using crane
crane push migration.tar registry.example.com/migrations/add-users:v1
```

## Proposed Enhancement

Add a `--push` flag (or separate `tern push` command) to upload OCI images directly to registries:

```bash
# Option A: Flag on build command
tern build \
  --database-url "$DATABASE_URL" \
  --description "Add users table" \
  --package-format oci \
  --push registry.example.com/migrations/add-users:v1

# Option B: Separate push command
tern push ./migration.tar registry.example.com/migrations/add-users:v1
```

## Technical Requirements

### OCI Distribution Specification

Registry communication follows the [OCI Distribution Specification](https://github.com/opencontainers/distribution-spec/blob/main/spec.md). The upload process requires:

1. **Check blob existence** - `HEAD /v2/<name>/blobs/<digest>`
2. **Initiate blob upload** - `POST /v2/<name>/blobs/uploads/`
3. **Upload blob content** - `PUT /v2/<name>/blobs/uploads/<session>?digest=<digest>`
4. **Upload manifest** - `PUT /v2/<name>/manifests/<reference>`

### Recommended Crate: `oci-distribution`

The [`oci-distribution`](https://crates.io/crates/oci-distribution) crate provides a complete client implementation for the OCI Distribution Specification:

```toml
[dependencies]
oci-distribution = "0.11"
```

Key features:
- Token-based and basic authentication
- Chunked uploads for large layers
- Manifest push/pull operations
- TLS support with custom CA certificates

### Authentication

Registries use various authentication methods:

| Method | Description | Example Registries |
|--------|-------------|-------------------|
| Anonymous | No authentication required | Public registries |
| Basic Auth | Username/password | Harbor, Artifactory |
| Bearer Token | OAuth2-style tokens | Docker Hub, GCR, ECR |
| Cloud IAM | Cloud provider identity | GCR (gcloud), ECR (aws) |

The implementation should support:

1. **Docker config file** (`~/.docker/config.json`) - Standard credential storage
2. **Environment variables** - `REGISTRY_USERNAME`, `REGISTRY_PASSWORD`
3. **Credential helpers** - `docker-credential-*` binaries for cloud providers

The [`docker-credential-rs`](https://crates.io/crates/docker-credential) crate can parse Docker config files.

## Implementation Plan

### Phase 1: Core Upload Functionality

1. Add `oci-distribution` dependency
2. Implement `RegistryClient` wrapper with:
   - Connection management
   - Retry logic with exponential backoff
   - Progress reporting
3. Add blob upload logic (check existence, chunked upload)
4. Add manifest upload logic

### Phase 2: Authentication

1. Parse `~/.docker/config.json` for stored credentials
2. Support environment variable credentials
3. Integrate credential helpers for cloud registries
4. Add `--username` and `--password` CLI flags

### Phase 3: CLI Integration

1. Add `--push <reference>` flag to `tern build`
2. Optionally add standalone `tern push` command
3. Add progress bars for upload status
4. Support `--insecure-registry` for HTTP registries

## API Design

### Rust API

```rust
use tern::db::compile::oci::{OciImageBuilder, RegistryClient, RegistryAuth};

// Build the image
let builder = OciImageBuilder::default();
let image = builder.build(&executable_path, &output_path, target)?;

// Push to registry
let client = RegistryClient::new(RegistryAuth::from_docker_config()?);
let reference = "registry.example.com/migrations/add-users:v1".parse()?;
client.push(&image, &reference).await?;
```

### CLI Interface

```
tern build [OPTIONS] --output <PATH>

Options:
    --push <REFERENCE>       Push image to registry after building
    --username <USER>        Registry username (or use REGISTRY_USERNAME env)
    --password <PASS>        Registry password (or use REGISTRY_PASSWORD env)
    --insecure-registry      Allow HTTP (non-TLS) registry connections
```

## Error Handling

The implementation should provide clear error messages for common issues:

| Error | Message | Help |
|-------|---------|------|
| Auth failure | `Registry authentication failed` | Check credentials in ~/.docker/config.json |
| Network error | `Failed to connect to registry` | Verify registry URL and network access |
| Permission denied | `Permission denied for repository` | Ensure push access to the repository |
| Blob upload failed | `Failed to upload layer` | Retry or check registry storage |

## Testing Strategy

1. **Unit tests** - Mock registry responses
2. **Integration tests** - Use [zot](https://zotregistry.io/) or [distribution](https://github.com/distribution/distribution) as local test registry
3. **E2E tests** - Push to actual registries in CI (Docker Hub, ghcr.io)

## Security Considerations

1. **Credential storage** - Never log or expose credentials
2. **TLS verification** - Require TLS by default, explicit opt-out for HTTP
3. **Digest verification** - Verify uploaded content matches expected digest
4. **Token scope** - Request minimal required permissions

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `oci-distribution` | 0.11 | OCI Distribution Spec client |
| `docker-credential` | 1.3 | Docker config parsing |
| `indicatif` | 0.17 | Progress bars (optional) |

## References

- [OCI Distribution Specification](https://github.com/opencontainers/distribution-spec)
- [OCI Image Specification](https://github.com/opencontainers/image-spec)
- [oci-distribution crate](https://crates.io/crates/oci-distribution)
- [Docker Registry HTTP API V2](https://docs.docker.com/registry/spec/api/)
