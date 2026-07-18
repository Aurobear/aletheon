//! Network policy for tool execution hardening (D1-T12).
//!
//! Constrains outbound network access during tool execution.
//! Applied per-turn by the executive when a tool attempts a network operation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Policy controlling which outbound network connections are permitted.
///
/// The production default is deny. Configuration must explicitly select
/// `allow` (optionally constrained by allow lists) before network is usable.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkPolicy {
    pub default_action: NetworkDefaultAction,
    /// If non-empty, only these hosts (or suffix patterns) are allowed.
    pub allow_hosts: Vec<String>,
    /// Explicitly denied hosts or suffix patterns (highest priority).
    pub deny_hosts: Vec<String>,
    /// Allowed URL schemes (e.g. `["https", "http"]`). Empty = no restriction.
    pub allow_protocols: Vec<String>,
    /// Allowed ports: exact (e.g. `"443"`) or ranges (e.g. `"8080-8090"`).
    /// Empty = no restriction.
    pub allow_ports: Vec<String>,
    /// Whether DNS resolution is permitted. When `false`, hostname-based URLs
    /// are rejected; only IP-based URLs are allowed.
    #[serde(default)]
    pub allow_dns: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDefaultAction {
    Allow,
    #[default]
    Deny,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            default_action: NetworkDefaultAction::Deny,
            allow_hosts: Vec::new(),
            deny_hosts: Vec::new(),
            allow_protocols: Vec::new(),
            allow_ports: Vec::new(),
            allow_dns: false,
        }
    }
}

impl NetworkPolicy {
    /// Check whether a URL is permitted under this policy.
    ///
    /// Returns `Ok(())` if allowed, `Err(String)` with a reason if denied.
    ///
    /// Priority: deny_hosts > allow_hosts > allow_protocols > allow_ports > allow_dns.
    pub fn allows_url(&self, url: &str) -> Result<(), String> {
        let (scheme, host, port) = Self::parse_url(url)?;

        // 1. Check deny_hosts (highest priority).
        if !self.deny_hosts.is_empty() && self.matches_any(&host, &self.deny_hosts) {
            return Err(format!("host '{host}' is in deny_hosts"));
        }

        if self.default_action == NetworkDefaultAction::Deny && self.allow_hosts.is_empty() {
            return Err("network access is denied by default".into());
        }

        // 2. Check allow_hosts.
        if !self.allow_hosts.is_empty() && !self.matches_any(&host, &self.allow_hosts) {
            return Err(format!("host '{host}' is not in allow_hosts"));
        }

        // 3. Check allow_protocols (scheme).
        if !self.allow_protocols.is_empty() {
            let scheme_lower = scheme.to_lowercase();
            if !self
                .allow_protocols
                .iter()
                .any(|p| p.to_lowercase() == scheme_lower)
            {
                return Err(format!("protocol '{scheme}' is not in allow_protocols"));
            }
        }

        // 4. Check allow_ports.
        if !self.allow_ports.is_empty() {
            let url_port = port.unwrap_or_else(|| Self::default_port_for_scheme(&scheme));
            if !self.port_matches(url_port, &self.allow_ports) {
                return Err(format!("port {url_port} is not in allow_ports"));
            }
        }

        // 5. Check allow_dns: when false, hostnames are rejected.
        if !self.allow_dns && !Self::looks_like_ip(&host) {
            return Err(format!(
                "DNS not allowed: host '{host}' is not an IP address"
            ));
        }

        Ok(())
    }

    /// Basic URL parser. Extracts scheme, host, and optional port.
    /// Returns Err if the URL is unparseable.
    fn parse_url(url: &str) -> Result<(String, String, Option<u16>), String> {
        // Split off the scheme.
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| format!("invalid URL: no '://' found in '{url}'"))?;

        let scheme = scheme.to_lowercase();

        // Find where the authority ends: at the first '/', '?', '#', or end of string.
        let authority_end = rest
            .find('/')
            .or_else(|| rest.find('?'))
            .or_else(|| rest.find('#'))
            .unwrap_or(rest.len());

        let authority = &rest[..authority_end];

        // Split host and port. Port comes after the last ':' unless it's IPv6.
        let (host, port) = if authority.starts_with('[') {
            // IPv6 address: [::1]:8080
            let close_bracket = authority
                .find(']')
                .ok_or_else(|| format!("invalid URL: unclosed '[' in authority '{authority}'"))?;
            let host = &authority[1..close_bracket];
            let port = if authority.len() > close_bracket + 1 {
                // Expect ":<port>"
                if authority.as_bytes()[close_bracket + 1] == b':' {
                    let port_str = &authority[close_bracket + 2..];
                    Some(
                        port_str
                            .parse::<u16>()
                            .map_err(|_| format!("invalid port number: '{port_str}'"))?,
                    )
                } else {
                    None
                }
            } else {
                None
            };
            (host.to_string(), port)
        } else {
            // Hostname or IPv4: host:port
            match authority.rsplit_once(':') {
                Some((host, port_str)) => {
                    let host = host.to_lowercase();
                    let port = port_str
                        .parse::<u16>()
                        .map_err(|_| format!("invalid port number: '{port_str}'"))?;
                    (host, Some(port))
                }
                None => (authority.to_lowercase(), None),
            }
        };

        Ok((scheme, host, port))
    }

    /// Check whether a host matches any pattern in the list.
    ///
    /// Patterns:
    /// - Exact: `example.com` matches `example.com`
    /// - Suffix: `.example.com` matches `sub.example.com` but not `example.com`
    fn matches_any(&self, host: &str, patterns: &[String]) -> bool {
        let host_lower = host.to_lowercase();
        patterns.iter().any(|p| {
            if p.starts_with('.') {
                // Suffix match: ".example.com" matches "sub.example.com"
                host_lower.ends_with(p.as_str())
                    && host_lower.len() > p.len()
                    && !host_lower[..host_lower.len() - p.len()].contains('.')
            } else {
                // Exact match (case-insensitive).
                p.to_lowercase() == host_lower
            }
        })
    }

    /// Check whether a port matches any entry in the allow list.
    /// Entries can be exact (`"443"`) or ranges (`"8080-8090"`).
    fn port_matches(&self, port: u16, allowed: &[String]) -> bool {
        allowed.iter().any(|entry| {
            if let Some((low, high)) = entry.split_once('-') {
                let low: u16 = low.parse().ok().unwrap_or(0);
                let high: u16 = high.parse().ok().unwrap_or(0);
                port >= low && port <= high
            } else {
                entry.parse::<u16>().ok() == Some(port)
            }
        })
    }

    /// Get the default port for a well-known scheme.
    fn default_port_for_scheme(scheme: &str) -> u16 {
        match scheme {
            "http" | "ws" => 80,
            "https" | "wss" => 443,
            "ftp" => 21,
            "ssh" => 22,
            _ => 0,
        }
    }

    /// Heuristic: does the host string look like an IP address?
    /// Accepts IPv4 (e.g. `192.168.1.1`) and IPv6 (e.g. `::1`, `fe80::1`).
    fn looks_like_ip(host: &str) -> bool {
        // IPv6: contains ':' and consists of hex digits, ':', and '.'.
        if host.contains(':') {
            return host
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        }
        // IPv4: four decimal octets separated by dots.
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() != 4 {
            return false;
        }
        parts
            .iter()
            .all(|p| p.parse::<u8>().is_ok() && (!p.starts_with('0') || p.len() == 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Default policy (deny) --

    #[test]
    fn explicit_allow_policy_allows_anything_when_allow_dns_true() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("https://example.com").is_ok());
        assert!(policy.allows_url("http://evil.com:8080/path?q=1").is_ok());
        assert!(policy.allows_url("ftp://192.168.1.1").is_ok());
    }

    #[test]
    fn default_denies_dns_when_allow_dns_is_false() {
        let policy = NetworkPolicy::default();
        assert!(!policy.allow_dns);
        assert!(policy.allows_url("http://93.184.216.34").is_err());
        assert!(policy.allows_url("http://example.com").is_err());
    }

    // -- allow_hosts --

    #[test]
    fn allow_hosts_exact_match() {
        let policy = NetworkPolicy {
            allow_hosts: vec!["example.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("https://example.com").is_ok());
        assert!(policy
            .allows_url("https://other.com")
            .unwrap_err()
            .contains("not in allow_hosts"));
    }

    #[test]
    fn allow_hosts_blocks_non_matching() {
        let policy = NetworkPolicy {
            allow_hosts: vec!["api.example.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("https://api.example.com").is_ok());
        assert!(policy.allows_url("https://example.com").is_err());
    }

    // -- deny_hosts --

    #[test]
    fn deny_hosts_takes_precedence_over_allow_hosts() {
        let policy = NetworkPolicy {
            allow_hosts: vec![".example.com".into()],
            deny_hosts: vec!["evil.example.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        // Allowed via suffix match.
        assert!(policy.allows_url("https://good.example.com").is_ok());
        // Denied explicitly, even though suffix would allow it.
        let err = policy.allows_url("https://evil.example.com").unwrap_err();
        assert!(err.contains("in deny_hosts"));
    }

    #[test]
    fn deny_hosts_exact_match() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            deny_hosts: vec!["evil.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        let err = policy.allows_url("https://evil.com").unwrap_err();
        assert!(err.contains("'evil.com' is in deny_hosts"));
    }

    // -- allow_protocols --

    #[test]
    fn allow_protocols_filters_by_scheme() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_protocols: vec!["https".into(), "wss".into()],
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("https://example.com").is_ok());
        assert!(policy
            .allows_url("http://example.com")
            .unwrap_err()
            .contains("not in allow_protocols"));
    }

    // -- allow_ports --

    #[test]
    fn allow_ports_exact_match() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_ports: vec!["8080".into()],
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("http://example.com:8080").is_ok());
        assert!(policy
            .allows_url("http://example.com:9090")
            .unwrap_err()
            .contains("not in allow_ports"));
    }

    #[test]
    fn allow_ports_range() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_ports: vec!["8000-9000".into()],
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("http://example.com:8000").is_ok());
        assert!(policy.allows_url("http://example.com:8500").is_ok());
        assert!(policy.allows_url("http://example.com:9000").is_ok());
        assert!(policy
            .allows_url("http://example.com:7999")
            .unwrap_err()
            .contains("not in allow_ports"));
    }

    #[test]
    fn allow_ports_uses_default_port_when_url_has_no_explicit_port() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_ports: vec!["443".into()],
            allow_dns: true,
            ..Default::default()
        };
        // https default is 443 — should pass.
        assert!(policy.allows_url("https://example.com").is_ok());
        // http default is 80 — should fail.
        assert!(policy.allows_url("http://example.com").is_err());
    }

    // -- allow_dns --

    #[test]
    fn allow_dns_false_blocks_hostnames() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_dns: false,
            ..Default::default()
        };
        assert!(policy.allows_url("http://93.184.216.34").is_ok());
        assert!(policy
            .allows_url("http://example.com")
            .unwrap_err()
            .contains("DNS not allowed"));
    }

    #[test]
    fn allow_dns_true_allows_hostnames() {
        let policy = NetworkPolicy {
            default_action: NetworkDefaultAction::Allow,
            allow_dns: true,
            ..Default::default()
        };
        assert!(policy.allows_url("http://example.com").is_ok());
    }

    // -- Malformed URLs --

    #[test]
    fn malformed_url_returns_error() {
        let policy = NetworkPolicy::default();
        assert!(policy.allows_url("not-a-url").is_err());
        assert!(policy.allows_url("://missing-scheme").is_err());
    }

    // -- Suffix matching --

    #[test]
    fn suffix_pattern_matches_subdomains_only() {
        let policy = NetworkPolicy {
            allow_hosts: vec![".example.com".into()],
            allow_dns: true,
            ..Default::default()
        };
        // Suffix ".example.com" should match sub.example.com
        assert!(policy.allows_url("https://sub.example.com").is_ok());
        // But NOT example.com itself.
        assert!(policy.allows_url("https://example.com").is_err());
        // And NOT deeper subdomains e.g. a.b.example.com
        // (suffix match only allows one level below)
        // Actually: the current implementation checks host ends with ".example.com"
        // and the remaining part does not contain ".". For "a.b.example.com":
        // ends with ".example.com" yes, remaining is "a.b" which contains "." -> false.
        // This is the expected behavior.
    }
}
