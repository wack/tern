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
//! # Feature Flags
//!
//! When both components are successfully compiled, the `embedded-wasi` feature
//! is enabled, which causes embedded.rs to include the compiled components.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Tell Cargo to rerun this script if certain files change
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=crates/tern-migration-runner/src/");
    println!("cargo:rerun-if-changed=crates/tern-migration-guest/src/");
    println!("cargo:rerun-if-changed=crates/tern-migration-wit/wit/");

    // Get the output directory for build artifacts
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    // Paths for the compiled WASI components
    let runner_wasm_path = out_dir.join("runner.wasm");
    let guest_wasm_path = out_dir.join("guest.wasm");

    // Check if we should attempt WASI compilation
    let wasi_available = is_wasi_target_available();

    if wasi_available {
        println!("cargo:warning=wasm32-wasip2 target available, attempting WASI compilation...");

        let runner_ok = compile_wasi_component(
            "tern-migration-runner",
            "crates/tern-migration-runner",
            &runner_wasm_path,
        )
        .is_ok();

        let guest_ok = compile_wasi_component(
            "tern-migration-guest",
            "crates/tern-migration-guest",
            &guest_wasm_path,
        )
        .is_ok();

        if runner_ok && guest_ok {
            println!("cargo:warning=WASI components compiled successfully!");
            println!("cargo:rustc-cfg=feature=\"embedded-wasi\"");
        } else {
            println!("cargo:warning=WASI compilation failed, falling back to cargo-based pipeline");
        }
    } else {
        println!("cargo:warning=wasm32-wasip2 target not available");
        println!("cargo:warning=Install with: rustup target add wasm32-wasip2");
        println!("cargo:warning=Falling back to cargo-based compilation pipeline");
    }

    // Export paths as environment variables for use in src/db/compile/embedded.rs
    // These are always set, but embedded.rs only uses them when embedded-wasi feature is enabled
    println!(
        "cargo:rustc-env=TERN_RUNNER_WASM_PATH={}",
        runner_wasm_path.display()
    );
    println!(
        "cargo:rustc-env=TERN_GUEST_WASM_PATH={}",
        guest_wasm_path.display()
    );
}

/// Check if the wasm32-wasip2 target is available.
fn is_wasi_target_available() -> bool {
    // Check environment variable to skip WASI compilation
    if env::var("TERN_SKIP_WASI_BUILD").is_ok() {
        println!("cargo:warning=TERN_SKIP_WASI_BUILD set, skipping WASI compilation");
        return false;
    }

    // Try to get the list of installed targets
    let output = match Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        Ok(output) => output,
        Err(_) => {
            // rustup not available, try cargo instead
            return check_cargo_target_available();
        }
    };

    if !output.status.success() {
        return check_cargo_target_available();
    }

    let installed = String::from_utf8_lossy(&output.stdout);
    installed.lines().any(|line| line.trim() == "wasm32-wasip2")
}

/// Fallback check using cargo to see if the target works.
fn check_cargo_target_available() -> bool {
    // Try a minimal cargo check to see if the target works
    let output = Command::new("cargo")
        .args([
            "check",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            "crates/tern-migration-wit/Cargo.toml",
        ])
        .output();

    matches!(output, Ok(o) if o.status.success())
}

/// Compile a crate to a WASI component.
///
/// # Arguments
///
/// * `crate_name` - Name of the crate to compile
/// * `crate_path` - Path to the crate directory
/// * `output_path` - Where to write the compiled .wasm file
fn compile_wasi_component(
    crate_name: &str,
    crate_path: &str,
    output_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:warning=Compiling {} to wasm32-wasip2...", crate_name);

    // Get the workspace root (build.rs runs from the package root)
    let manifest_path = format!("{}/Cargo.toml", crate_path);

    // Build the crate for wasm32-wasip2 target
    let output = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            &manifest_path,
        ])
        .env("CARGO_TARGET_DIR", "target") // Use shared target dir
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("cargo:warning=Failed to compile {}: {}", crate_name, stderr);
        return Err(format!("Failed to compile {} to WASI", crate_name).into());
    }

    // Find the compiled wasm file
    // The crate name with dashes becomes underscores in the binary
    let binary_name = crate_name.replace('-', "_");
    let wasm_file =
        PathBuf::from("target/wasm32-wasip2/release").join(format!("{}.wasm", binary_name));

    if !wasm_file.exists() {
        println!(
            "cargo:warning=Compiled wasm not found at: {}",
            wasm_file.display()
        );
        return Err(format!("Compiled wasm not found at: {}", wasm_file.display()).into());
    }

    // Copy to output location
    std::fs::copy(&wasm_file, output_path)?;

    println!(
        "cargo:warning=Successfully compiled {} ({} bytes)",
        crate_name,
        std::fs::metadata(output_path)?.len()
    );

    Ok(())
}
