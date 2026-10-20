//! Port security validation to prevent SSRF attacks.
//!
//! This module enforces strict port validation to prevent Server-Side Request
//! Forgery (SSRF) attacks where an attacker might attempt to proxy requests to
//! sensitive internal services.

use std::ops::RangeInclusive;

/// Development servers typically run on ports 3000-9999.
pub const ALLOWED_PORT_RANGE: RangeInclusive<u16> = 3000..=9999;

/// These ports are commonly used by system services and databases.
pub const BLOCKED_PORTS: &[u16] = &[
    22,    // SSH
    23,    // Telnet
    25,    // SMTP
    110,   // POP3
    143,   // IMAP
    443,   // HTTPS (should not proxy to https on localhost)
    3306,  // MySQL
    5432,  // PostgreSQL
    6379,  // Redis
    27017, // MongoDB
];

pub fn validate_port(port: u16) -> Result<(), String> {
    // Check if port is in the blocked list first (even if in allowed range)
    if BLOCKED_PORTS.contains(&port) {
        return Err(format!(
            "Port {port} is blocked for security reasons. This port is commonly used by system \
             services and cannot be proxied to."
        ));
    }

    // Check if port is within allowed range
    if !ALLOWED_PORT_RANGE.contains(&port) {
        return Err(format!(
            "Port {port} is outside the allowed range ({}-{}). Only development server ports are \
             permitted.",
            ALLOWED_PORT_RANGE.start(),
            ALLOWED_PORT_RANGE.end()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ports() {
        // Test common development ports
        assert!(validate_port(3000).is_ok());
        assert!(validate_port(3001).is_ok());
        assert!(validate_port(8080).is_ok());
        assert!(validate_port(8000).is_ok());
        assert!(validate_port(9999).is_ok());
    }

    #[test]
    fn test_blocked_ports() {
        // Test that all blocked ports are rejected
        for &port in BLOCKED_PORTS {
            let result = validate_port(port);
            assert!(
                result.is_err(),
                "Port {port} should be blocked but was allowed"
            );
            let err = result.unwrap_err();
            assert!(
                err.contains("blocked for security reasons"),
                "Error message should mention security: {err}"
            );
        }
    }

    #[test]
    fn test_ports_below_range() {
        // Test ports below the allowed range
        assert!(validate_port(0).is_err());
        assert!(validate_port(80).is_err());
        assert!(validate_port(1000).is_err());
        assert!(validate_port(2999).is_err());

        let err = validate_port(1000).unwrap_err();
        assert!(
            err.contains("outside the allowed range"),
            "Error should mention allowed range: {err}"
        );
    }

    #[test]
    fn test_ports_above_range() {
        // Test ports above the allowed range
        assert!(validate_port(10000).is_err());
        assert!(validate_port(20000).is_err());
        assert!(validate_port(65535).is_err());

        let err = validate_port(10000).unwrap_err();
        assert!(
            err.contains("outside the allowed range"),
            "Error should mention allowed range: {err}"
        );
    }

    #[test]
    fn test_edge_cases() {
        // Test boundary conditions
        assert!(validate_port(3000).is_ok(), "Lower bound should be allowed");
        assert!(validate_port(9999).is_ok(), "Upper bound should be allowed");
        assert!(
            validate_port(2999).is_err(),
            "Just below lower bound should be rejected"
        );
        assert!(
            validate_port(10000).is_err(),
            "Just above upper bound should be rejected"
        );
    }

    #[test]
    fn test_blocked_port_within_range() {
        // Port 3306 (MySQL) is within 3000-9999 but should still be blocked
        let result = validate_port(3306);
        assert!(
            result.is_err(),
            "Blocked port within allowed range should still be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("blocked for security reasons"),
            "Should prioritize block list over range check: {err}"
        );
    }
}
