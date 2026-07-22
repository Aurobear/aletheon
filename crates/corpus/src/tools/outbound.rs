//! Minimum shared outbound HTTP policy used by protocol adapters.
//!
//! Protocol semantics stay in the owning adapter. This module only owns
//! endpoint identity, address-class enforcement, redirects, and timeout caps.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const MAX_OUTBOUND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointTrust {
    PublicInternet,
    LocalLoopback,
    TrustedPrivateNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutboundPolicyError {
    InvalidEndpoint,
    SchemeDenied,
    AuthorityDenied,
    AddressDenied,
    ResolutionFailed,
}

impl std::fmt::Display for OutboundPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidEndpoint => "invalid outbound endpoint",
            Self::SchemeDenied => "outbound endpoint scheme is denied",
            Self::AuthorityDenied => "outbound endpoint authority is denied",
            Self::AddressDenied => "outbound endpoint resolved to a prohibited address",
            Self::ResolutionFailed => "outbound endpoint resolution failed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for OutboundPolicyError {}

#[derive(Debug, Clone)]
pub(crate) struct EndpointPolicy {
    trust: EndpointTrust,
}

impl EndpointPolicy {
    pub(crate) fn public() -> Self {
        Self {
            trust: EndpointTrust::PublicInternet,
        }
    }

    pub(crate) fn local_loopback() -> Self {
        Self {
            trust: EndpointTrust::LocalLoopback,
        }
    }

    pub(crate) fn trusted_private_network() -> Self {
        Self {
            trust: EndpointTrust::TrustedPrivateNetwork,
        }
    }

    pub(crate) fn validate_identity(
        &self,
        endpoint: &str,
    ) -> Result<reqwest::Url, OutboundPolicyError> {
        let url =
            reqwest::Url::parse(endpoint).map_err(|_| OutboundPolicyError::InvalidEndpoint)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(OutboundPolicyError::SchemeDenied);
        }
        let host = url.host_str().ok_or(OutboundPolicyError::InvalidEndpoint)?;
        if url.username() != "" || url.password().is_some() {
            return Err(OutboundPolicyError::AuthorityDenied);
        }
        match self.trust {
            EndpointTrust::LocalLoopback
                if host != "localhost" && host.parse::<IpAddr>().is_err() =>
            {
                return Err(OutboundPolicyError::AuthorityDenied);
            }
            EndpointTrust::TrustedPrivateNetwork if host == "localhost" => {
                return Err(OutboundPolicyError::AuthorityDenied);
            }
            _ => {}
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            self.validate_ip(ip)?;
        }
        Ok(url)
    }

    pub(crate) async fn approve(
        &self,
        endpoint: &str,
    ) -> Result<reqwest::Url, OutboundPolicyError> {
        let url = self.validate_identity(endpoint)?;
        let host = url.host_str().ok_or(OutboundPolicyError::InvalidEndpoint)?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            self.validate_ip(ip)?;
            return Ok(url);
        }
        let port = url
            .port_or_known_default()
            .ok_or(OutboundPolicyError::InvalidEndpoint)?;
        let addresses: Vec<_> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| OutboundPolicyError::ResolutionFailed)?
            .collect();
        if addresses.is_empty() {
            return Err(OutboundPolicyError::ResolutionFailed);
        }
        for address in addresses {
            self.validate_ip(address.ip())?;
        }
        Ok(url)
    }

    pub(crate) fn client(&self, timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
        let timeout = timeout.min(MAX_OUTBOUND_TIMEOUT);
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10).min(timeout))
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(GuardedResolver {
                policy: self.clone(),
            }))
            .build()
    }

    fn validate_ip(&self, ip: IpAddr) -> Result<(), OutboundPolicyError> {
        let allowed = match self.trust {
            EndpointTrust::LocalLoopback => ip.is_loopback(),
            EndpointTrust::PublicInternet => is_public(ip),
            EndpointTrust::TrustedPrivateNetwork => is_remote_non_metadata(ip),
        };
        allowed
            .then_some(())
            .ok_or(OutboundPolicyError::AddressDenied)
    }
}

#[derive(Debug)]
struct GuardedResolver {
    policy: EndpointPolicy,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let policy = self.policy.clone();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| policy.validate_ip(address.ip()).is_err())
            {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "outbound address denied",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254))
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

fn is_remote_non_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && a != 0
                && a < 240
                && !(a == 169 && b == 254)
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && (ip.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_policy_denies_sensitive_address_classes() {
        let policy = EndpointPolicy::public();
        for ip in [
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert_eq!(
                policy.validate_ip(ip.parse().unwrap()),
                Err(OutboundPolicyError::AddressDenied)
            );
        }
        assert!(policy.validate_ip("8.8.8.8".parse().unwrap()).is_ok());
        assert!(policy
            .validate_ip("2606:4700:4700::1111".parse().unwrap())
            .is_ok());
    }

    #[tokio::test]
    async fn loopback_requires_explicit_local_trust_and_authority() {
        assert!(EndpointPolicy::public()
            .approve("http://127.0.0.1:3131/mcp")
            .await
            .is_err());
        assert!(EndpointPolicy::local_loopback()
            .approve("http://127.0.0.1:3131/mcp")
            .await
            .is_ok());
        assert!(EndpointPolicy::local_loopback()
            .approve("http://example.com/mcp")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn guarded_resolver_rechecks_addresses_after_dns() {
        let resolver = GuardedResolver {
            policy: EndpointPolicy::public(),
        };
        let error = match resolver.resolve("localhost".parse().unwrap()).await {
            Ok(_) => panic!("public resolver accepted loopback DNS result"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("outbound address denied"));
    }

    #[tokio::test]
    async fn guarded_client_does_not_follow_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let policy = EndpointPolicy::local_loopback();
        let response = policy
            .client(Duration::from_secs(1))
            .unwrap()
            .get(format!("http://{address}/start"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    }

    #[test]
    fn credentials_in_authority_and_non_http_schemes_are_denied() {
        let policy = EndpointPolicy::public();
        assert_eq!(
            policy.validate_identity("https://user:secret@example.com"),
            Err(OutboundPolicyError::AuthorityDenied)
        );
        assert_eq!(
            policy.validate_identity("file:///etc/passwd"),
            Err(OutboundPolicyError::SchemeDenied)
        );
    }

    #[test]
    fn private_trust_allows_remote_private_but_never_loopback_or_metadata() {
        let policy = EndpointPolicy::trusted_private_network();
        assert!(policy.validate_ip("100.64.1.2".parse().unwrap()).is_ok());
        assert!(policy.validate_ip("192.168.1.2".parse().unwrap()).is_ok());
        assert!(policy.validate_ip("fd00::2".parse().unwrap()).is_ok());
        assert!(policy.validate_ip("127.0.0.1".parse().unwrap()).is_err());
        assert!(policy
            .validate_ip("169.254.169.254".parse().unwrap())
            .is_err());
        assert!(policy.validate_ip("fe80::1".parse().unwrap()).is_err());
    }
}
