//! Import management for Rust code generation.
//!
//! This module handles collecting and generating use statements for SeaORM entities.

use std::collections::{BTreeSet, HashMap};

use super::type_mapping::RustImport;

/// Collects and organizes imports for generated Rust code.
#[derive(Debug, Clone, Default)]
pub struct ImportCollector {
    /// Standard library imports.
    std_imports: BTreeSet<String>,
    /// External crate imports (crate -> items).
    external_imports: HashMap<String, BTreeSet<String>>,
    /// Required SeaORM features.
    required_features: BTreeSet<String>,
}

impl ImportCollector {
    /// Creates a new import collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a collector with the standard SeaORM entity prelude.
    pub fn with_sea_orm_prelude() -> Self {
        let mut collector = Self::new();
        collector.add_sea_orm_prelude();
        collector
    }

    /// Adds the standard SeaORM entity prelude import.
    pub fn add_sea_orm_prelude(&mut self) {
        self.external_imports
            .entry("sea_orm::entity::prelude".to_string())
            .or_default()
            .insert("*".to_string());
    }

    /// Adds a Rust import.
    pub fn add_import(&mut self, import: &RustImport) {
        self.external_imports
            .entry(import.module.clone())
            .or_default()
            .insert(import.name.clone());
    }

    /// Adds multiple imports.
    pub fn add_imports(&mut self, imports: &[RustImport]) {
        for import in imports {
            self.add_import(import);
        }
    }

    /// Adds a required feature flag.
    pub fn add_feature(&mut self, feature: &str) {
        self.required_features.insert(feature.to_string());
    }

    /// Adds multiple feature flags.
    pub fn add_features(&mut self, features: &[&str]) {
        for feature in features {
            self.required_features.insert((*feature).to_string());
        }
    }

    /// Merges another collector into this one.
    pub fn merge(&mut self, other: &ImportCollector) {
        self.std_imports.extend(other.std_imports.iter().cloned());
        for (module, items) in &other.external_imports {
            self.external_imports
                .entry(module.clone())
                .or_default()
                .extend(items.iter().cloned());
        }
        self.required_features
            .extend(other.required_features.iter().cloned());
    }

    /// Returns the required SeaORM features.
    pub fn required_features(&self) -> Vec<String> {
        self.required_features.iter().cloned().collect()
    }

    /// Generates the import block as a string.
    pub fn generate(&self) -> String {
        let mut lines = Vec::new();

        // Standard library imports (sorted)
        for import in &self.std_imports {
            lines.push(format!("use std::{import};"));
        }

        if !self.std_imports.is_empty() && !self.external_imports.is_empty() {
            lines.push(String::new());
        }

        // External imports (sorted by crate name)
        let mut sorted_modules: Vec<_> = self.external_imports.keys().collect();
        sorted_modules.sort();

        for module in sorted_modules {
            let items = &self.external_imports[module];

            // Special handling for wildcard imports
            if items.contains("*") {
                lines.push(format!("use {module}::*;"));
                continue;
            }

            // Format imports
            if items.len() == 1 {
                let item = items.iter().next().unwrap();
                lines.push(format!("use {module}::{item};"));
            } else {
                let mut sorted_items: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
                sorted_items.sort();
                let items_str = sorted_items.join(", ");
                lines.push(format!("use {module}::{{{items_str}}};"));
            }
        }

        lines.join("\n")
    }

    /// Generates a feature flags comment block.
    pub fn generate_features_comment(&self) -> Option<String> {
        if self.required_features.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("//! Required SeaORM features:".to_string());
        for feature in &self.required_features {
            lines.push(format!("//! - `{feature}`"));
        }

        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_collector() {
        let collector = ImportCollector::new();
        assert_eq!(collector.generate(), "");
    }

    #[test]
    fn test_sea_orm_prelude() {
        let collector = ImportCollector::with_sea_orm_prelude();
        let imports = collector.generate();
        assert!(imports.contains("use sea_orm::entity::prelude::*;"));
    }

    #[test]
    fn test_add_import() {
        let mut collector = ImportCollector::new();
        collector.add_import(&RustImport::new("chrono", "DateTime"));
        let imports = collector.generate();
        assert!(imports.contains("use chrono::DateTime;"));
    }

    #[test]
    fn test_multiple_imports_same_module() {
        let mut collector = ImportCollector::new();
        collector.add_import(&RustImport::new("chrono", "DateTime"));
        collector.add_import(&RustImport::new("chrono", "FixedOffset"));
        let imports = collector.generate();
        assert!(imports.contains("use chrono::{DateTime, FixedOffset};"));
    }

    #[test]
    fn test_multiple_modules() {
        let mut collector = ImportCollector::new();
        collector.add_import(&RustImport::new("chrono", "DateTime"));
        collector.add_import(&RustImport::new("uuid", "Uuid"));
        let imports = collector.generate();
        assert!(imports.contains("use chrono::DateTime;"));
        assert!(imports.contains("use uuid::Uuid;"));
    }

    #[test]
    fn test_add_features() {
        let mut collector = ImportCollector::new();
        collector.add_feature("with-chrono");
        collector.add_feature("with-uuid");
        let features = collector.required_features();
        assert!(features.contains(&"with-chrono".to_string()));
        assert!(features.contains(&"with-uuid".to_string()));
    }

    #[test]
    fn test_generate_features_comment() {
        let mut collector = ImportCollector::new();
        collector.add_feature("with-chrono");
        let comment = collector.generate_features_comment().unwrap();
        assert!(comment.contains("with-chrono"));
    }

    #[test]
    fn test_merge() {
        let mut collector1 = ImportCollector::new();
        collector1.add_import(&RustImport::new("chrono", "DateTime"));
        collector1.add_feature("with-chrono");

        let mut collector2 = ImportCollector::new();
        collector2.add_import(&RustImport::new("uuid", "Uuid"));
        collector2.add_feature("with-uuid");

        collector1.merge(&collector2);

        let imports = collector1.generate();
        assert!(imports.contains("chrono"));
        assert!(imports.contains("uuid"));

        let features = collector1.required_features();
        assert!(features.contains(&"with-chrono".to_string()));
        assert!(features.contains(&"with-uuid".to_string()));
    }
}
