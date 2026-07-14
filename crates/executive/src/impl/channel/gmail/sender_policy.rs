//! Defensive Gmail sender authentication and principal binding policy.

use fabric::PrincipalId;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_HEADERS: usize = 200;
const MAX_HEADER_NAME: usize = 128;
const MAX_HEADER_VALUE: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationRequirement {
    SpfOrDkim,
    Spf,
    Dkim,
    SpfAndDkim,
}

#[derive(Debug, Clone)]
pub struct GmailSenderPolicy {
    pub principal: PrincipalId,
    pub version: u64,
    pub allowed_addresses: HashSet<String>,
    pub allowed_domains: HashSet<String>,
    pub trusted_authserv_ids: HashSet<String>,
    pub authentication: AuthenticationRequirement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGmailSender {
    pub principal: PrincipalId,
    pub address: String,
    pub policy_version: u64,
    pub evidence_hash: String,
    pub spf_aligned: bool,
    pub dkim_aligned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderPolicyError {
    Denied,
    MalformedHeaders,
    AmbiguousFrom,
    UntrustedAuthenticationResults,
    AuthenticationFailed,
}

impl GmailSenderPolicy {
    pub fn verify(
        &self,
        headers: &[GmailHeader],
    ) -> Result<VerifiedGmailSender, SenderPolicyError> {
        validate_policy(self)?;
        validate_headers(headers)?;
        let from_values = values(headers, "from");
        if from_values.len() != 1 {
            return Err(SenderPolicyError::AmbiguousFrom);
        }
        let address = parse_single_mailbox(from_values[0])?;
        let domain = address.rsplit_once('@').unwrap().1;
        if !self.allowed_addresses.contains(&address) && !self.allowed_domains.contains(domain) {
            return Err(SenderPolicyError::Denied);
        }

        let authentication_headers = values(headers, "authentication-results");
        let first = authentication_headers
            .first()
            .ok_or(SenderPolicyError::UntrustedAuthenticationResults)?;
        let authserv = first
            .split(';')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(SenderPolicyError::UntrustedAuthenticationResults)?
            .to_ascii_lowercase();
        if !self.trusted_authserv_ids.contains(&authserv) {
            return Err(SenderPolicyError::UntrustedAuthenticationResults);
        }
        if authentication_headers.iter().skip(1).any(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|candidate| candidate.trim().eq_ignore_ascii_case(&authserv))
        }) {
            return Err(SenderPolicyError::UntrustedAuthenticationResults);
        }
        let lower = first.to_ascii_lowercase();
        let spf_aligned = result_passes_aligned_domain(&lower, "spf=pass", "smtp.mailfrom", domain);
        let dkim_aligned = result_passes_aligned_domain(&lower, "dkim=pass", "header.d", domain);
        let accepted = match self.authentication {
            AuthenticationRequirement::SpfOrDkim => spf_aligned || dkim_aligned,
            AuthenticationRequirement::Spf => spf_aligned,
            AuthenticationRequirement::Dkim => dkim_aligned,
            AuthenticationRequirement::SpfAndDkim => spf_aligned && dkim_aligned,
        };
        if !accepted {
            return Err(SenderPolicyError::AuthenticationFailed);
        }
        let evidence = format!(
            "{}\n{}\n{}\n{}\n{}",
            self.principal.0, self.version, address, authserv, lower
        );
        Ok(VerifiedGmailSender {
            principal: self.principal.clone(),
            address,
            policy_version: self.version,
            evidence_hash: format!("{:x}", Sha256::digest(evidence.as_bytes())),
            spf_aligned,
            dkim_aligned,
        })
    }
}

fn validate_policy(policy: &GmailSenderPolicy) -> Result<(), SenderPolicyError> {
    let valid_addresses = policy.allowed_addresses.iter().all(|address| {
        parse_single_mailbox(address).is_ok_and(|canonical| canonical == *address)
            && address.bytes().all(|byte| byte.is_ascii())
    });
    let valid_domains = policy
        .allowed_domains
        .iter()
        .all(|domain| valid_domain(domain));
    let valid_authserv = policy.trusted_authserv_ids.iter().all(|value| {
        !value.is_empty()
            && value.len() <= 253
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    });
    if policy.principal.0.is_empty()
        || policy.version == 0
        || (!valid_addresses || !valid_domains || !valid_authserv)
        || policy.trusted_authserv_ids.is_empty()
    {
        Err(SenderPolicyError::Denied)
    } else {
        Ok(())
    }
}

fn validate_headers(headers: &[GmailHeader]) -> Result<(), SenderPolicyError> {
    if headers.is_empty() || headers.len() > MAX_HEADERS {
        return Err(SenderPolicyError::MalformedHeaders);
    }
    if headers.iter().any(|header| {
        header.name.is_empty()
            || header.name.len() > MAX_HEADER_NAME
            || header.value.len() > MAX_HEADER_VALUE
            || !header
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || header.value.contains(['\r', '\n', '\0'])
    }) {
        return Err(SenderPolicyError::MalformedHeaders);
    }
    Ok(())
}

fn values<'a>(headers: &'a [GmailHeader], name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
        .collect()
}

fn parse_single_mailbox(value: &str) -> Result<String, SenderPolicyError> {
    if !value.is_ascii() || value.contains(',') || value.contains(['\r', '\n', '\0']) {
        return Err(SenderPolicyError::AmbiguousFrom);
    }
    if value.matches('<').count() > 1 || value.matches('>').count() > 1 {
        return Err(SenderPolicyError::AmbiguousFrom);
    }
    let candidate = match (value.rfind('<'), value.rfind('>')) {
        (Some(start), Some(end)) if start < end && value[end + 1..].trim().is_empty() => {
            &value[start + 1..end]
        }
        (None, None) => value,
        _ => return Err(SenderPolicyError::AmbiguousFrom),
    }
    .trim()
    .to_ascii_lowercase();
    let Some((local, domain)) = candidate.rsplit_once('@') else {
        return Err(SenderPolicyError::AmbiguousFrom);
    };
    if local.is_empty()
        || local.len() > 64
        || !local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&byte))
        || !valid_domain(domain)
    {
        return Err(SenderPolicyError::AmbiguousFrom);
    }
    Ok(candidate)
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain == domain.to_ascii_lowercase()
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn result_passes_aligned_domain(
    value: &str,
    result: &str,
    property: &str,
    from_domain: &str,
) -> bool {
    value.split(';').any(|clause| {
        let clause = clause.trim();
        clause.starts_with(result)
            && clause.split_whitespace().any(|part| {
                part.strip_prefix(&format!("{property}="))
                    .and_then(|identity| {
                        identity
                            .rsplit_once('@')
                            .map(|(_, domain)| domain)
                            .or(Some(identity))
                    })
                    .is_some_and(|domain| {
                        domain
                            .trim_matches(['<', '>'])
                            .eq_ignore_ascii_case(from_domain)
                    })
            })
    })
}
