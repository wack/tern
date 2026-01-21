//! Build script for Tern.
//!
//! This script is responsible for compiling the WebAssembly components that
//! are embedded in the Tern binary. These components enable the WASI-based
//! migration compilation pipeline.
//!
//! # Components Built
//!
//! 1. **tern-migration-runner** → `runner.wasm`
//!    - Target: `wasm32-wasip2`
//!    - A WASI CLI application that orchestrates migration execution
//!
//! 2. **tern-migration-guest** → `guest.wasm`
//!    - Target: `wasm32-wasip2`
//!    - A generic migration executor that imports SQL data
//!
//! # Environment Variables Set
//!
//! - `TERN_RUNNER_WASM_PATH`: Path to compiled runner.wasm
//! - `TERN_GUEST_WASM_PATH`: Path to compiled guest.wasm
//!
//! # Current Status
//!
//! The WASI compilation is currently disabled because:
//! - `tern-migration-runner` needs to be rewritten for WASI (remove wasmtime dependency)
//! - `tern-migration-guest` needs to be rewritten for WASI
//!
//! Once those crates are updated, uncomment the compilation code below.

use std::env;
use std::path::PathBuf;

fn main() {
    // Tell Cargo to rerun this script if certain files change
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=crates/tern-migration-runner/src/");
    println!("cargo:rerun-if-changed=crates/tern-migration-guest/src/");

    // Get the output directory for build artifacts
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    // Placeholder paths for the Wasm components
    // These will be set properly once the WASI rewrite is complete
    let runner_wasm_path = out_dir.join("runner.wasm");
    let guest_wasm_path = out_dir.join("guest.wasm");

    // Export paths as environment variables for use in src/db/compile/embedded.rs
    // NOTE: These files don't exist yet - the embedded.rs module handles this gracefully
    println!(
        "cargo:rustc-env=TERN_RUNNER_WASM_PATH={}",
        runner_wasm_path.display()
    );
    println!(
        "cargo:rustc-env=TERN_GUEST_WASM_PATH={}",
        guest_wasm_path.display()
    );

    // =========================================================================
    // WASI Component Compilation (currently disabled)
    // =========================================================================
    //
    // Once the runner and guest crates are rewritten for WASI, uncomment and
    // complete this section. The general approach is:
    //
    // 1. Check if wasm32-wasip2 target is available
    // 2. Compile each crate to that target
    // 3. Optionally, use wasm-tools to convert to component format
    //
    // ```
    // compile_wasi_component(
    //     "tern-migration-runner",
    //     "crates/tern-migration-runner",
    //     &runner_wasm_path,
    // );
    //
    // compile_wasi_component(
    //     "tern-migration-guest",
    //     "crates/tern-migration-guest",
    //     &guest_wasm_path,
    // );
    // ```

    // For now, just print a note about the missing components
    if !runner_wasm_path.exists() {
        println!(
            "cargo:warning=Runner WASM component not available (WASI rewrite pending): {}",
            runner_wasm_path.display()
        );
    }

    if !guest_wasm_path.exists() {
        println!(
            "cargo:warning=Guest WASM component not available (WASI rewrite pending): {}",
            guest_wasm_path.display()
        );
    }
}

/// Compile a crate to a WASI component.
///
/// This function will be used once the runner and guest crates are rewritten
/// to target WASI.
///
/// # Arguments
///
/// * `crate_name` - Name of the crate to compile
/// * `crate_path` - Path to the crate directory
/// * `output_path` - Where to write the compiled .wasm file
#[allow(dead_code)]
fn compile_wasi_component(
    crate_name: &str,
    crate_path: &str,
    output_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    println!("cargo:warning=Compiling {} to WASI...", crate_name);

    // Build the crate for wasm32-wasip2 target
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            &format!("{}/Cargo.toml", crate_path),
        ])
        .status()?;

    if !status.success() {
        return Err(format!("Failed to compile {} to WASI", crate_name).into());
    }

    // Find the compiled wasm file
    let wasm_file = PathBuf::from(crate_path)
        .join("target/wasm32-wasip2/release")
        .join(format!("{}.wasm", crate_name.replace('-', "_")));

    if !wasm_file.exists() {
        return Err(format!("Compiled wasm not found at: {}", wasm_file.display()).into());
    }

    // Copy to output location
    std::fs::copy(&wasm_file, output_path)?;

    println!(
        "cargo:warning=Successfully compiled {} to {}",
        crate_name,
        output_path.display()
    );

    Ok(())
}

/// Convert a WASI module to a component using wasm-tools.
///
/// WASI Preview 2 components may need additional processing after
/// compilation. This function handles that conversion.
///
/// # Arguments
///
/// * `module_path` - Path to the input .wasm module
/// * `component_path` - Path to write the output component
#[allow(dead_code)]
fn convert_to_component(
    module_path: &std::path::Path,
    component_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    // Use wasm-tools to convert module to component
    // This may be needed depending on how the crates are compiled
    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            module_path.to_str().unwrap(),
            "-o",
            component_path.to_str().unwrap(),
        ])
        .status()?;

    if !status.success() {
        return Err(format!("Failed to convert {} to component", module_path.display()).into());
    }

    Ok(())
}
