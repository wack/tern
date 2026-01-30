//! Python import management.
//!
//! This module handles collecting and organizing Python imports for generated code.

use std::collections::{BTreeMap, BTreeSet};

use super::type_mapping::PythonImport;

/// Collects and organizes Python imports for code generation.
#[derive(Debug, Default)]
pub struct ImportCollector {
    /// Standard library imports grouped by module.
    /// Using BTreeMap/BTreeSet for deterministic ordering.
    stdlib: BTreeMap<String, BTreeSet<String>>,
    /// Third-party imports (sqlmodel, sqlalchemy, pydantic).
    third_party: BTreeMap<String, BTreeSet<String>>,
    /// TYPE_CHECKING imports (for forward references).
    type_checking: BTreeMap<String, BTreeSet<String>>,
}

impl ImportCollector {
    /// Creates a new empty import collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a Python import to the collector.
    pub fn add(&mut self, import: &PythonImport) {
        let category = categorize_module(&import.module);
        let map = match category {
            ModuleCategory::Stdlib => &mut self.stdlib,
            ModuleCategory::ThirdParty => &mut self.third_party,
        };

        map.entry(import.module.clone())
            .or_default()
            .insert(import.name.clone());
    }

    /// Adds multiple imports to the collector.
    pub fn add_all(&mut self, imports: &[PythonImport]) {
        for import in imports {
            self.add(import);
        }
    }

    /// Adds a TYPE_CHECKING import (for forward references).
    /// Used for relationship generation to avoid circular imports.
    #[allow(dead_code)]
    pub fn add_type_checking(&mut self, module: &str, name: &str) {
        self.type_checking
            .entry(module.to_string())
            .or_default()
            .insert(name.to_string());
    }

    /// Adds the core SQLModel import.
    pub fn add_sqlmodel(&mut self) {
        self.add(&PythonImport::new("sqlmodel", "SQLModel"));
    }

    /// Adds the SQLModel Field import.
    pub fn add_field(&mut self) {
        self.add(&PythonImport::new("sqlmodel", "Field"));
    }

    /// Adds the SQLModel Relationship import.
    /// Used when generate_relationships config option is enabled.
    #[allow(dead_code)]
    pub fn add_relationship(&mut self) {
        self.add(&PythonImport::new("sqlmodel", "Relationship"));
    }

    /// Checks if there are any TYPE_CHECKING imports.
    /// Used for relationship generation to determine if TYPE_CHECKING block is needed.
    #[allow(dead_code)]
    pub fn has_type_checking(&self) -> bool {
        !self.type_checking.is_empty()
    }

    /// Generates the import statements as a formatted string.
    pub fn generate(&self) -> String {
        let mut lines = Vec::new();

        // Standard library imports
        if !self.stdlib.is_empty() {
            for (module, names) in &self.stdlib {
                lines.push(format_import_line(module, names));
            }
        }

        // Third-party imports (with blank line separator if needed)
        if !self.third_party.is_empty() {
            if !self.stdlib.is_empty() {
                lines.push(String::new());
            }
            for (module, names) in &self.third_party {
                lines.push(format_import_line(module, names));
            }
        }

        // TYPE_CHECKING block
        if !self.type_checking.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("if TYPE_CHECKING:".to_string());
            for (module, names) in &self.type_checking {
                let import_line = format_import_line(module, names);
                lines.push(format!("    {import_line}"));
            }
            // Ensure typing.TYPE_CHECKING is imported
            self.ensure_type_checking_imported(&mut lines);
        }

        lines.join("\n")
    }

    /// Ensures TYPE_CHECKING is imported from typing if we have a TYPE_CHECKING block.
    fn ensure_type_checking_imported(&self, lines: &mut [String]) {
        // Check if TYPE_CHECKING is already in typing imports
        if let Some(typing_names) = self.stdlib.get("typing") {
            if typing_names.contains("TYPE_CHECKING") {
                return;
            }
        }

        // Find the typing import line and add TYPE_CHECKING
        for line in lines.iter_mut() {
            if line.starts_with("from typing import ") {
                // Add TYPE_CHECKING to the existing import
                if !line.contains("TYPE_CHECKING") {
                    let insert_pos = "from typing import ".len();
                    line.insert_str(insert_pos, "TYPE_CHECKING, ");
                }
                return;
            }
        }

        // No typing import found, need to add one at the beginning
        // This case should be handled by the caller adding TYPE_CHECKING import explicitly
    }

    /// Merges another ImportCollector into this one.
    pub fn merge(&mut self, other: &ImportCollector) {
        for (module, names) in &other.stdlib {
            self.stdlib
                .entry(module.clone())
                .or_default()
                .extend(names.iter().cloned());
        }
        for (module, names) in &other.third_party {
            self.third_party
                .entry(module.clone())
                .or_default()
                .extend(names.iter().cloned());
        }
        for (module, names) in &other.type_checking {
            self.type_checking
                .entry(module.clone())
                .or_default()
                .extend(names.iter().cloned());
        }
    }
}

/// Module category for import grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleCategory {
    Stdlib,
    ThirdParty,
}

/// Categorizes a module as stdlib or third-party.
fn categorize_module(module: &str) -> ModuleCategory {
    // Known third-party modules
    const THIRD_PARTY: &[&str] = &["sqlmodel", "sqlalchemy", "pydantic", "fastapi", "starlette"];

    if THIRD_PARTY.iter().any(|&m| module.starts_with(m)) {
        ModuleCategory::ThirdParty
    } else {
        ModuleCategory::Stdlib
    }
}

/// Formats a single import line.
fn format_import_line(module: &str, names: &BTreeSet<String>) -> String {
    let names_str: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

    if names_str.len() == 1 {
        format!("from {module} import {}", names_str[0])
    } else {
        // Sort for deterministic output
        let mut sorted_names = names_str;
        sorted_names.sort();

        // Check if we need multi-line format
        let single_line = format!("from {module} import {}", sorted_names.join(", "));
        if single_line.len() <= 88 {
            // PEP 8 line length
            single_line
        } else {
            // Multi-line format
            let mut lines = vec![format!("from {module} import (")];
            for name in sorted_names {
                lines.push(format!("    {name},"));
            }
            lines.push(")".to_string());
            lines.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_simple_import() {
        let mut collector = ImportCollector::new();
        collector.add(&PythonImport::new("datetime", "datetime"));

        let output = collector.generate();
        assert!(output.contains("from datetime import datetime"));
    }

    #[test]
    fn test_add_multiple_from_same_module() {
        let mut collector = ImportCollector::new();
        collector.add(&PythonImport::new("datetime", "datetime"));
        collector.add(&PythonImport::new("datetime", "date"));
        collector.add(&PythonImport::new("datetime", "timedelta"));

        let output = collector.generate();
        assert!(output.contains("from datetime import"));
        assert!(output.contains("date"));
        assert!(output.contains("datetime"));
        assert!(output.contains("timedelta"));
    }

    #[test]
    fn test_stdlib_before_third_party() {
        let mut collector = ImportCollector::new();
        collector.add(&PythonImport::new("sqlmodel", "SQLModel"));
        collector.add(&PythonImport::new("datetime", "datetime"));

        let output = collector.generate();
        let datetime_pos = output.find("datetime").unwrap();
        let sqlmodel_pos = output.find("sqlmodel").unwrap();
        assert!(datetime_pos < sqlmodel_pos);
    }

    #[test]
    fn test_type_checking_block() {
        let mut collector = ImportCollector::new();
        collector.add(&PythonImport::new("typing", "TYPE_CHECKING"));
        collector.add_type_checking(".user", "User");

        let output = collector.generate();
        assert!(output.contains("if TYPE_CHECKING:"));
        assert!(output.contains("from .user import User"));
    }

    #[test]
    fn test_sqlmodel_imports() {
        let mut collector = ImportCollector::new();
        collector.add_sqlmodel();
        collector.add_field();

        let output = collector.generate();
        assert!(output.contains("from sqlmodel import"));
        assert!(output.contains("Field"));
        assert!(output.contains("SQLModel"));
    }

    #[test]
    fn test_merge_collectors() {
        let mut collector1 = ImportCollector::new();
        collector1.add(&PythonImport::new("datetime", "datetime"));

        let mut collector2 = ImportCollector::new();
        collector2.add(&PythonImport::new("datetime", "date"));
        collector2.add(&PythonImport::new("sqlmodel", "SQLModel"));

        collector1.merge(&collector2);
        let output = collector1.generate();

        assert!(output.contains("date"));
        assert!(output.contains("datetime"));
        assert!(output.contains("SQLModel"));
    }

    #[test]
    fn test_deterministic_output() {
        // Run multiple times to verify ordering is consistent
        for _ in 0..5 {
            let mut collector = ImportCollector::new();
            collector.add(&PythonImport::new("uuid", "UUID"));
            collector.add(&PythonImport::new("datetime", "datetime"));
            collector.add(&PythonImport::new("typing", "Any"));
            collector.add(&PythonImport::new("sqlmodel", "SQLModel"));
            collector.add(&PythonImport::new("sqlmodel", "Field"));

            let output = collector.generate();

            // Check order: stdlib (datetime, typing, uuid) then third-party (sqlmodel)
            let datetime_pos = output.find("from datetime").unwrap();
            let typing_pos = output.find("from typing").unwrap();
            let uuid_pos = output.find("from uuid").unwrap();
            let sqlmodel_pos = output.find("from sqlmodel").unwrap();

            assert!(datetime_pos < typing_pos);
            assert!(typing_pos < uuid_pos);
            assert!(uuid_pos < sqlmodel_pos);
        }
    }

    #[test]
    fn test_categorize_module() {
        assert_eq!(categorize_module("datetime"), ModuleCategory::Stdlib);
        assert_eq!(categorize_module("typing"), ModuleCategory::Stdlib);
        assert_eq!(categorize_module("uuid"), ModuleCategory::Stdlib);
        assert_eq!(categorize_module("sqlmodel"), ModuleCategory::ThirdParty);
        assert_eq!(categorize_module("sqlalchemy"), ModuleCategory::ThirdParty);
        assert_eq!(
            categorize_module("sqlalchemy.types"),
            ModuleCategory::ThirdParty
        );
    }
}
