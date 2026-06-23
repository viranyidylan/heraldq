use http_acl::HttpAcl;
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

pub fn policy() -> HttpAcl {
    HttpAcl::builder()
        .ip_acl_default(true)
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
