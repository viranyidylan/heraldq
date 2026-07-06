use axum::BoxError;
use http_acl::HttpAcl;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::SocketAddr;
use std::sync::Arc;
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

/// Egress IP policy: allow any global IP, deny the entire non-global set
/// (loopback, RFC1918, link-local + cloud metadata, CGNAT, IPv6 ULA,
/// documentation, reserved, ...) sourced from the stdlib's `is_global`.
///
/// These are the only two fields `is_ip_allowed` reads, and `is_ip_allowed`
/// is the only ACL surface we consume — scheme and credential rejection live
/// in `validate_destination`, not here.
pub fn policy() -> HttpAcl {
    HttpAcl::builder()
        .non_global_ip_ranges(false) // deny every non-global range
        .ip_acl_default(true) // allow any global IP not explicitly denied
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

/// Reject the whole resolution if *any* resolved address is denied. A split DNS
/// answer mixing a public and a private IP is treated as hostile — this is the
/// check that defeats DNS rebinding at connect time, after registration.
fn screen_addrs(acl: &HttpAcl, addrs: &[SocketAddr]) -> Result<(), SsrfError> {
    for a in addrs {
        if acl.is_ip_allowed(&a.ip()).is_denied() {
            return Err(SsrfError::BlockedAddress);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct SecureResolver {
    pub acl: Arc<HttpAcl>,
}

impl Resolve for SecureResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let acl = self.acl.clone(); // refcount bump, not a deep copy of the ACL
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as BoxError)?
                .collect();

            screen_addrs(&acl, &addrs)?;

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

    fn sa(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 443)
    }

    #[test]
    fn screen_addrs_allows_all_public() {
        let acl = policy();
        assert_eq!(screen_addrs(&acl, &[sa("1.1.1.1"), sa("93.184.216.34")]), Ok(()));
    }

    #[test]
    fn screen_addrs_rejects_all_private() {
        let acl = policy();
        assert_eq!(
            screen_addrs(&acl, &[sa("127.0.0.1")]),
            Err(SsrfError::BlockedAddress)
        );
    }

    #[test]
    fn screen_addrs_rejects_metadata_ip() {
        let acl = policy();
        assert_eq!(
            screen_addrs(&acl, &[sa("169.254.169.254")]),
            Err(SsrfError::BlockedAddress)
        );
    }

    // The DNS-rebinding defense: one public IP alongside one private IP must
    // sink the *entire* resolution, not just filter the bad address out.
    #[test]
    fn screen_addrs_rejects_split_answer() {
        let acl = policy();
        assert_eq!(
            screen_addrs(&acl, &[sa("1.1.1.1"), sa("169.254.169.254")]),
            Err(SsrfError::BlockedAddress)
        );
    }

    // End-to-end: a blocked destination must surface as a *recognizable* SSRF
    // block through reqwest's error wrapping — dispatch's control flow hinges
    // on `ssrf_block()` returning true (else it silently retries a blocked
    // host instead of dead-lettering it). `localhost` resolves offline to
    // 127.0.0.1/::1 and, being a hostname not an IP literal, actually goes
    // through our resolver — an IP literal would skip resolution entirely.
    #[tokio::test]
    async fn blocked_host_surfaces_as_recognized_ssrf_block() {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(SecureResolver { acl: Arc::new(policy()) }))
            .build()
            .unwrap();

        let err = client
            .get("https://localhost/")
            .send()
            .await
            .expect_err("request to a blocked host must fail");

        assert!(err.ssrf_block(), "expected an SSRF block, got: {err:?}");
        assert_eq!(err.get_ssrf(), Some(&SsrfError::BlockedAddress));
    }
}
