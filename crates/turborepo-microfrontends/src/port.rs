pub const MIN_PORT: u16 = 3000;
pub const MAX_PORT: u16 = 8000;
const PORT_RANGE: u16 = MAX_PORT - MIN_PORT;

pub fn generate_port_from_name(name: &str) -> u16 {
    let mut hash: i32 = 0;
    for c in name.chars() {
        let code = i32::try_from(u32::from(c)).expect("char::MAX is less than 2^31");
        hash = (hash << 5).overflowing_sub(hash).0.overflowing_add(code).0;
    }
    let hash = hash.abs_diff(0);
    let port = hash % u32::from(PORT_RANGE);
    MIN_PORT + u16::try_from(port).expect("u32 modulo a u16 number will be a valid u16")
}

pub fn parse_port_from_host(host: &str) -> Option<u16> {
    // Remove protocol if present
    let without_protocol = if let Some(idx) = host.find("://") {
        &host[idx + 3..]
    } else {
        host
    };

    // Extract port after the last colon
    if let Some(colon_idx) = without_protocol.rfind(':')
        && let Ok(port) = without_protocol[colon_idx + 1..].parse::<u16>()
    {
        return Some(port);
    }

    None
}

#[cfg(test)]
mod test {
    use std::char;

    use super::*;

    #[test]
    fn test_generate_port() {
        assert_eq!(generate_port_from_name("test-450"), 7724);
    }

    #[test]
    fn test_generate_port_deterministic() {
        let a = generate_port_from_name("my-vercel-project");
        let b = generate_port_from_name("my-vercel-project");
        assert_eq!(a, b);
        assert!((MIN_PORT..MAX_PORT).contains(&a));
    }

    #[test]
    fn test_generate_port_range() {
        for name in ["a", "web", "docs", "my-very-long-application-name"] {
            let port = generate_port_from_name(name);
            assert!(
                (MIN_PORT..MAX_PORT).contains(&port),
                "port {port} out of range for name '{name}'"
            );
        }
    }

    #[test]
    fn test_char_as_i32() {
        let max_char = u32::from(char::MAX);
        assert!(
            i32::try_from(max_char).is_ok(),
            "max char should fit in i32"
        );
    }

    #[test]
    fn test_parse_port_from_host_with_port() {
        assert_eq!(parse_port_from_host("localhost:3000"), Some(3000));
    }

    #[test]
    fn test_parse_port_from_host_with_protocol() {
        assert_eq!(parse_port_from_host("http://localhost:3000"), Some(3000));
    }

    #[test]
    fn test_parse_port_from_host_without_port() {
        assert_eq!(parse_port_from_host("localhost"), None);
    }
}
