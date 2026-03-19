//! Import management for Drizzle ORM code generation.
//!
//! This module handles collecting and organizing TypeScript imports for generated code.
//! All column builder imports come from `drizzle-orm/pg-core`, while relation helpers
//! come from `drizzle-orm`.

use std::collections::BTreeSet;

/// Collects and organizes TypeScript imports for Drizzle code generation.
#[derive(Debug, Default, Clone)]
pub struct ImportCollector {
    /// Imports from "drizzle-orm/pg-core" (pgTable, serial, text, etc.).
    pg_core: BTreeSet<String>,
    /// Imports from "drizzle-orm" (relations, etc.).
    drizzle_orm: BTreeSet<String>,
}

impl ImportCollector {
    /// Creates a new empty import collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a "drizzle-orm/pg-core" import.
    pub fn add_pg_core(&mut self, name: &str) {
        self.pg_core.insert(name.to_string());
    }

    /// Adds multiple "drizzle-orm/pg-core" imports.
    pub fn add_pg_core_all(&mut self, names: &[String]) {
        for name in names {
            self.pg_core.insert(name.clone());
        }
    }

    /// Adds a "drizzle-orm" import.
    pub fn add_drizzle_orm(&mut self, name: &str) {
        self.drizzle_orm.insert(name.to_string());
    }

    /// Adds the pgTable import.
    pub fn add_pg_table(&mut self) {
        self.add_pg_core("pgTable");
    }

    /// Adds the pgSchema import.
    pub fn add_pg_schema(&mut self) {
        self.add_pg_core("pgSchema");
    }

    /// Adds the primaryKey import (for composite PKs).
    pub fn add_primary_key(&mut self) {
        self.add_pg_core("primaryKey");
    }

    /// Adds the unique import (for composite unique constraints).
    pub fn add_unique(&mut self) {
        self.add_pg_core("unique");
    }

    /// Adds the index import.
    pub fn add_index(&mut self) {
        self.add_pg_core("index");
    }

    /// Adds the relations import.
    pub fn add_relations(&mut self) {
        self.add_drizzle_orm("relations");
    }

    /// Merges another ImportCollector into this one.
    pub fn merge(&mut self, other: &ImportCollector) {
        self.pg_core.extend(other.pg_core.iter().cloned());
        self.drizzle_orm.extend(other.drizzle_orm.iter().cloned());
    }

    /// Generates the import statements as formatted TypeScript code.
    pub fn generate(&self) -> String {
        let mut lines = Vec::new();

        if !self.pg_core.is_empty() {
            let names: Vec<&str> = self.pg_core.iter().map(|s| s.as_str()).collect();
            lines.push(format_import_line(&names, "drizzle-orm/pg-core"));
        }

        if !self.drizzle_orm.is_empty() {
            let names: Vec<&str> = self.drizzle_orm.iter().map(|s| s.as_str()).collect();
            lines.push(format_import_line(&names, "drizzle-orm"));
        }

        lines.join("\n")
    }

    /// Returns true if there are no imports.
    pub fn is_empty(&self) -> bool {
        self.pg_core.is_empty() && self.drizzle_orm.is_empty()
    }
}

/// Formats a single import line.
fn format_import_line(names: &[&str], module: &str) -> String {
    let joined = names.join(", ");
    let single_line = format!("import {{ {joined} }} from \"{module}\";");

    if single_line.len() <= 100 {
        single_line
    } else {
        // Multi-line format
        let mut lines = vec![format!("import {{")];
        for name in names {
            lines.push(format!("  {name},"));
        }
        lines.push(format!("}} from \"{module}\";"));
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_collector() {
        let collector = ImportCollector::new();
        assert!(collector.is_empty());
        assert_eq!(collector.generate(), "");
    }

    #[test]
    fn test_pg_core_imports() {
        let mut collector = ImportCollector::new();
        collector.add_pg_table();
        collector.add_pg_core("serial");
        collector.add_pg_core("text");

        let output = collector.generate();
        assert!(output.contains("import {"));
        assert!(output.contains("pgTable"));
        assert!(output.contains("serial"));
        assert!(output.contains("text"));
        assert!(output.contains("drizzle-orm/pg-core"));
    }

    #[test]
    fn test_drizzle_orm_imports() {
        let mut collector = ImportCollector::new();
        collector.add_relations();

        let output = collector.generate();
        assert!(output.contains("relations"));
        assert!(output.contains("drizzle-orm"));
        assert!(!output.contains("pg-core"));
    }

    #[test]
    fn test_both_import_sources() {
        let mut collector = ImportCollector::new();
        collector.add_pg_table();
        collector.add_pg_core("text");
        collector.add_relations();

        let output = collector.generate();
        // pg-core line should come first (alphabetical order of module path)
        let pg_core_pos = output.find("drizzle-orm/pg-core").unwrap();
        let drizzle_pos = output
            .find("\"drizzle-orm\"")
            .or_else(|| output.find("from \"drizzle-orm\""))
            .unwrap();
        assert!(pg_core_pos < drizzle_pos);
    }

    #[test]
    fn test_deterministic_ordering() {
        for _ in 0..5 {
            let mut collector = ImportCollector::new();
            collector.add_pg_core("varchar");
            collector.add_pg_core("text");
            collector.add_pg_core("serial");
            collector.add_pg_core("integer");
            collector.add_pg_table();

            let output = collector.generate();
            // BTreeSet ensures alphabetical order
            let integer_pos = output.find("integer").unwrap();
            let pg_table_pos = output.find("pgTable").unwrap();
            let serial_pos = output.find("serial").unwrap();
            let text_pos = output.find("text").unwrap();
            let varchar_pos = output.find("varchar").unwrap();

            assert!(integer_pos < pg_table_pos);
            assert!(pg_table_pos < serial_pos);
            assert!(serial_pos < text_pos);
            assert!(text_pos < varchar_pos);
        }
    }

    #[test]
    fn test_merge_collectors() {
        let mut c1 = ImportCollector::new();
        c1.add_pg_core("text");
        c1.add_pg_table();

        let mut c2 = ImportCollector::new();
        c2.add_pg_core("integer");
        c2.add_relations();

        c1.merge(&c2);

        let output = c1.generate();
        assert!(output.contains("text"));
        assert!(output.contains("integer"));
        assert!(output.contains("pgTable"));
        assert!(output.contains("relations"));
    }

    #[test]
    fn test_deduplication() {
        let mut collector = ImportCollector::new();
        collector.add_pg_core("text");
        collector.add_pg_core("text");
        collector.add_pg_core("text");

        let output = collector.generate();
        // "text" should appear only once in the import
        let count = output.matches("text").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_add_pg_core_all() {
        let mut collector = ImportCollector::new();
        collector.add_pg_core_all(&["text".to_string(), "integer".to_string()]);

        assert!(!collector.is_empty());
        let output = collector.generate();
        assert!(output.contains("text"));
        assert!(output.contains("integer"));
    }
}
