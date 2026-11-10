pub mod strings;

#[cfg(test)]
mod tests {
    #[test]
    fn snap() {
        insta::assert_snapshot!("x");
    }
}
