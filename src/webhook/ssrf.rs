use axum::BoxError;
use http_acl::{HttpAcl, IpNet};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::SocketAddr;
use url::Host;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SsrfError {
    #[error("destination is not a valid URL")]
    BadUrl,
    #[error("destination must use https")]
    NotHttps,
    #[error("destination must not embed credentials")]
    HasCredentials,
    #[error("destination address is not allowed")]
    BlockedAddress,
    #[error("destination has no host")]
    MissingHost,
}

/// Walks an error and its `source()` chain looking for a concrete `T`.
///
/// reqwest wraps DNS resolver errors through several layers before they
/// surface as a generic `reqwest::Error`, but the original error is still
/// in there — this recovers it.
pub fn find_in_chain<'a, T: std::error::Error + 'static>(
    err: &'a (dyn std::error::Error + 'static),
) -> Option<&'a T> {
    let mut cur = Some(err);
    while let Some(e) = cur {
        if let Some(t) = e.downcast_ref::<T>() {
            return Some(t);
        }
        cur = e.source();
    }
    None
}

/// Lets callers ask a `reqwest::Error` "were you actually an SSRF block in disguise?"
/// the same way reqwest's own `is_connect()` / `is_timeout()` work.
pub trait SsrfCheck {
    fn ssrf_block(&self) -> bool;
    fn get_ssrf(&self) -> Option<&SsrfError>;
}

impl SsrfCheck for reqwest::Error {
    fn ssrf_block(&self) -> bool {
        find_in_chain::<SsrfError>(self).is_some()
    }
    fn get_ssrf(&self) -> Option<&SsrfError> {
        find_in_chain::<SsrfError>(self)
    }
}

pub fn policy() -> HttpAcl {
    HttpAcl::builder()
        .https(true)
        // The crux: deny every non-global range (private, loopback, link-local,
        // CGNAT, ULA, documentation, etc.). This replaces a hand-rolled CIDR table.
        .non_global_ip_ranges(false)
        // "Any destination": allow hosts and global IPs unless explicitly denied.
        .host_acl_default(true)
        .ip_acl_default(true)
        // Ports: lock to 80/443. For truly-any port, use .port_acl_default(true)
        // instead and drop the two allow ranges below.
        .port_acl_default(false)
        // Belt-and-suspenders explicit denies. non_global already covers these,
        // but being explicit documents intent and survives config drift.
        .add_denied_ip_range("169.254.0.0/16".parse::<IpNet>().unwrap()) // link-local + cloud metadata
        .expect("valid cidr")
        .add_denied_ip_range("100.64.0.0/10".parse::<IpNet>().unwrap()) // CGNAT
        .expect("valid cidr")
        .add_denied_ip_range("fc00::/7".parse::<IpNet>().unwrap()) // IPv6 ULA
        .expect("valid cidr")
        .build()
}

pub fn validate_destination(acl: &HttpAcl, raw: &str) -> Result<(), SsrfError> {
    let url = reqwest::Url::parse(raw).map_err(|_| SsrfError::BadUrl)?;

    if url.scheme() != "https" {
        return Err(SsrfError::NotHttps);
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(SsrfError::HasCredentials)
    }

    match url.host() {
        Some(Host::Ipv4(ip)) => {
            if acl.is_ip_allowed(&ip.into()).is_denied() {
                return Err(SsrfError::BlockedAddress);
            }   
        }
        Some(Host::Ipv6(ip)) => {
            if acl.is_ip_allowed(&ip.into()).is_denied() {
                return Err(SsrfError::BlockedAddress);
            }   
        }
        Some(Host::Domain(_)) => (),
        None => return Err(SsrfError::MissingHost),
    }
    Ok(())
}

#[derive(Clone)]
pub struct SecureResolver {
    pub acl: HttpAcl,
}

impl Resolve for SecureResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.acl.clone(); 
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as BoxError)?
                .collect();

            // If any returned address is disallowed, reject the
            // entire resolution. Treats split results as hostile
            for a in &addrs {
                if policy.is_ip_allowed(&a.ip()).is_denied() {
                    // return Err(format!("found disallowed IP in hostname, rejecting request").into())
                    return Err(SsrfError::BlockedAddress.into())
                }

            }

            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_allows_public_denies_private() {
        let acl = policy();
        assert!(acl.is_ip_allowed(&"1.1.1.1".parse().unwrap()).is_allowed());
        assert!(acl.is_ip_allowed(&"127.0.0.1".parse().unwrap()).is_denied());
    }

    #[test]
    fn validate_destination_blocks_ip_literal_evasions() {
        let acl = policy();

        // The classic targets attackers reach for.
        let blocked = [
            "https://169.254.169.254/latest/meta-data/",   // cloud metadata endpoint
            "https://2130706433/",                         // decimal for 127.0.0.1
            "https://0x7f.0.0.1/",                         // hex octet
            "https://0177.0.0.1/",                         // octal octet
            "https://10.0.0.1/",                           // RFC1918 private
            "https://192.168.1.1/",                        // RFC1918 private
            "https://[::1]/",                              // IPv6 loopback
            "https://[fe80::1]/",                          // IPv6 link-local
        ];

        for raw in blocked {
            assert_eq!(
                validate_destination(&acl, raw),
                Err(SsrfError::BlockedAddress),
                "expected {raw} to be blocked",
            );
        }
    }

    #[test]
    fn validate_destination_rejects_bad_destinations() {
        let acl = policy();

        let cases = vec![
            ("/api/v1/users",                Err(SsrfError::BadUrl)),
            ("http://my-website.com",        Err(SsrfError::NotHttps)),
            ("gopher://surprise-attack",     Err(SsrfError::NotHttps)),
            ("https://user:pw@example.com",  Err(SsrfError::HasCredentials)),
            ("https://2130706433/",          Err(SsrfError::BlockedAddress)),
            ("https://example.com",          Ok(()) ),
            ("https://93.184.216.34",        Ok(()) ),
        ];

        for (url, expected) in cases {
            assert_eq!(
                validate_destination(&acl, url),
                expected
            )
        }
    }
}
