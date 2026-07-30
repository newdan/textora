use std::net::IpAddr;

use reqwest::Url;

use crate::SyncError;

#[derive(Clone, Debug)]
pub struct LoopbackEndpoint {
    url: Url,
    display: String,
}

impl LoopbackEndpoint {
    pub fn parse(candidate: &str) -> Result<Self, SyncError> {
        let url = Url::parse(candidate).map_err(|error| SyncError::InvalidEndpoint {
            reason: format!("malformed URL: {error}"),
        })?;

        if url.scheme() != "http" {
            return Err(SyncError::InvalidEndpoint { reason: "only http is permitted".to_owned() });
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(SyncError::InvalidEndpoint {
                reason: "userinfo is not permitted".to_owned(),
            });
        }

        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(SyncError::InvalidEndpoint {
                reason: "endpoint must not contain a path, query, or fragment".to_owned(),
            });
        }

        let host = url
            .host_str()
            .ok_or_else(|| SyncError::InvalidEndpoint { reason: "host is required".to_owned() })?;
        if !is_loopback_host(host) {
            return Err(SyncError::InvalidEndpoint {
                reason: "host must be localhost or a loopback IP".to_owned(),
            });
        }

        let mut normalized = url;
        normalized.set_path("");
        let display = normalized.as_str().trim_end_matches('/').to_owned();
        Ok(Self { url: normalized, display })
    }

    pub(crate) fn join(&self, path: &str) -> Result<Url, SyncError> {
        if !path.starts_with('/') {
            return Err(SyncError::InvalidEndpoint {
                reason: "request path must start with '/'".to_owned(),
            });
        }
        self.url.join(path).map_err(|error| SyncError::InvalidEndpoint {
            reason: format!("invalid request path: {error}"),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.display
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host_without_ipv6_brackets = host.trim_matches(['[', ']']);
    host_without_ipv6_brackets.eq_ignore_ascii_case("localhost")
        || host_without_ipv6_brackets.parse::<IpAddr>().is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::LoopbackEndpoint;

    #[test]
    fn accepts_supported_loopback_hosts_and_normalizes_trailing_slash() {
        for candidate in ["http://127.0.0.1:8384", "http://localhost:8384/", "http://[::1]:8384"] {
            let endpoint = LoopbackEndpoint::parse(candidate)
                .unwrap_or_else(|error| panic!("{candidate} should parse: {error:?}"));
            assert!(endpoint.as_str().starts_with("http://"));
            assert!(!endpoint.as_str().ends_with('/'));
        }
    }

    #[test]
    fn rejects_non_loopback_or_non_root_endpoints() {
        for candidate in [
            "https://127.0.0.1:8384",
            "http://192.168.1.10:8384",
            "http://example.test:8384",
            "http://user:pass@127.0.0.1:8384",
            "http://127.0.0.1:8384/api",
            "http://127.0.0.1:8384?x=1",
            "http://127.0.0.1:8384#fragment",
        ] {
            assert!(LoopbackEndpoint::parse(candidate).is_err(), "accepted {candidate}");
        }
    }
}
