pub mod cli;
pub mod db;

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
