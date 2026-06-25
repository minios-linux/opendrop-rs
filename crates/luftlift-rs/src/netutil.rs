//! Network interface helpers — IPv6 address resolution for binding to awdl0.
//!
//! On Linux, reads `/proc/net/if_inet6` to find an interface's IPv6 address.
//! This is what opendrop's `AirDropUtil.get_ip_for_interface` does (via
//! `netifaces` on the Python side).

use std::net::Ipv6Addr;

/// Find the first IPv6 address on the named interface (typically the
/// link-local `fe80::` address on awdl0).
pub fn get_ipv6_for_interface(name: &str) -> Option<Ipv6Addr> {
    let content = std::fs::read_to_string("/proc/net/if_inet6").ok()?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 6 && parts[5] == name {
            return parse_inet6_addr(parts[0]);
        }
    }
    None
}

/// Parse a `/proc/net/if_inet6` hex address (32 hex chars, no colons)
/// into an `Ipv6Addr`.
fn parse_inet6_addr(hex: &str) -> Option<Ipv6Addr> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(Ipv6Addr::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inet6_addr_lo() {
        // ::1 as it appears in /proc/net/if_inet6
        let addr = parse_inet6_addr("00000000000000000000000000000001").unwrap();
        assert_eq!(addr, Ipv6Addr::LOCALHOST);
    }

    #[test]
    fn parse_inet6_addr_fe80() {
        // fe80::200:ff:fe12:3456 as 32 hex chars (no colons).
        let addr = parse_inet6_addr("fe80000000000000020000fffe123456").unwrap();
        assert!(addr.is_unicast_link_local());
    }

    #[test]
    fn parse_inet6_addr_rejects_short_hex() {
        assert!(parse_inet6_addr("abc").is_none());
    }

    #[test]
    fn get_ipv6_for_loopback_if_present() {
        // `lo` always has ::1 on Linux.
        if let Some(addr) = get_ipv6_for_interface("lo") {
            assert_eq!(addr, Ipv6Addr::LOCALHOST);
        }
    }
}
