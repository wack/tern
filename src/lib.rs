pub mod cli;
pub mod db;
pub mod mcp;
pub mod util;

// Re-export macros from tern-ddl for use in the main crate
pub use tern_ddl::assert_enum_char_roundtrip;
pub use tern_ddl::impl_char_enum;
pub use tern_ddl::impl_sql_enum;
pub use tern_ddl::impl_str_enum;
pub use tern_ddl::impl_str_from_enum;

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        // This test exists to ensure the test infrastructure works
        // before any real code is written.
        let expected = 2 + 2;
        assert_eq!(expected, 4);
    }
}
