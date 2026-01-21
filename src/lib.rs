pub mod cli;
pub mod db;

// =============================================================================
// Macros for Reducing Code Duplication
// =============================================================================

/// Implements `as_char()` method and `TryFrom<char>` trait for enums that map to single characters.
///
/// This macro generates:
/// - `as_char(&self) -> char` method returning the character for each variant
/// - `TryFrom<char>` implementation parsing characters back to variants
/// - Error handling with the specified error type
///
/// The error type must be a tuple struct that can be constructed with a single `char` argument.
///
/// # Example
///
/// ```ignore
/// pub enum MyEnum {
///     VariantA,
///     VariantB,
/// }
///
/// impl_char_enum!(MyEnum, MyError, [
///     VariantA => 'a',
///     VariantB => 'b',
/// ]);
/// ```
#[macro_export]
macro_rules! impl_char_enum {
    (
        $enum_type:ty,
        $error_type:path,
        [
            $($variant:ident => $char:expr),+ $(,)?
        ]
    ) => {
        impl $enum_type {
            /// Returns the single-character code used by PostgreSQL.
            #[must_use]
            pub const fn as_char(&self) -> char {
                match self {
                    $(Self::$variant => $char),+
                }
            }
        }

        impl TryFrom<char> for $enum_type {
            type Error = $error_type;

            fn try_from(c: char) -> Result<Self, Self::Error> {
                match c {
                    $($char => Ok(Self::$variant)),+,
                    _ => Err($error_type(c)),
                }
            }
        }
    };
}

/// Implements `as_sql()` method that returns SQL keywords for enum variants.
///
/// This macro generates:
/// - `as_sql(&self) -> &'static str` method
/// - Returns the SQL keyword for each variant
///
/// # Example
///
/// ```ignore
/// impl_sql_enum!(MyEnum, [
///     Cascade => "CASCADE",
///     Restrict => "RESTRICT",
/// ]);
/// ```
#[macro_export]
macro_rules! impl_sql_enum {
    (
        $enum_type:ty,
        [
            $($variant:ident => $sql:expr),+ $(,)?
        ]
    ) => {
        impl $enum_type {
            /// Returns the SQL keyword for this variant.
            #[must_use]
            pub const fn as_sql(&self) -> &'static str {
                match self {
                    $(Self::$variant => $sql),+
                }
            }
        }
    };
}

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
