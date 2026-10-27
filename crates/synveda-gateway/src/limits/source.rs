//! The network source a pre-authentication budget is charged to.
//!
//! The socket peer is the source unless it is a configured trusted proxy, in
//! which case the nearest untrusted `X-Forwarded-For` hop is. A forwarded
//! address from an untrusted peer is ignored, because any client can write
//! the header.

use std::net::IpAddr;

use axum::http::HeaderMap;
use ipnet::IpNet;

/// At most this many configured proxy networks.
pub(crate) const MAX_TRUSTED_PROXIES: usize = 64;
/// Forwarded hops examined from the right before giving up on attribution.
const MAX_FORWARDED_HOPS: usize = 16;

/// A network source, keyed so that one IPv6 /64 is one source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey([u8; 16]);

/// The source a request is attributed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Address(SourceKey),
    /// No peer address, or a trusted proxy that did not name a readable hop
    /// within the bound.
    Unattributed,
}

/// The configured proxy networks whose forwarded header is believed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedProxies(Vec<IpNet>);

/// Loopback, private and link-local networks: where a reverse proxy or
/// ingress in front of a self-hosted gateway connects from, and never a
/// public client address.
const PRIVATE_NETWORKS: &str =
    "127.0.0.0/8 ::1/128 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16 fc00::/7 fe80::/10";

impl Default for TrustedProxies {
    fn default() -> Self {
        Self::parse(PRIVATE_NETWORKS).unwrap_or_else(|_| Self(Vec::new()))
    }
}

impl TrustedProxies {
    /// Parses comma- or whitespace-separated addresses and CIDR networks.
    ///
    /// # Errors
    ///
    /// A message naming the bound or the malformed position, never the value.
    pub fn parse(value: &str) -> Result<Self, String> {
        let mut networks = Vec::new();
        for (position, item) in value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|item| !item.is_empty())
            .enumerate()
        {
            if networks.len() == MAX_TRUSTED_PROXIES {
                return Err(format!(
                    "SYNVEDA_TRUSTED_PROXIES names more than {MAX_TRUSTED_PROXIES} networks"
                ));
            }
            let network = item
                .parse::<IpNet>()
                .or_else(|_| item.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| {
                    format!(
                        "SYNVEDA_TRUSTED_PROXIES entry {} is not an address or CIDR network",
                        position + 1
                    )
                })?;
            networks.push(network.trunc());
        }
        Ok(Self(networks))
    }

    /// Whether any forwarded address is believed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, address: IpAddr) -> bool {
        let address = canonical(address);
        self.0.iter().any(|network| network.contains(&address))
    }
}

/// Attributes one request to a source.
///
/// When every hop is trusted, the hop nearest the peer is the client: private
/// clients share the default trusted networks with proxies, and only the
/// trusted peer wrote that hop.
pub(crate) fn attribute(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted: &TrustedProxies,
) -> Source {
    let Some(peer) = peer else {
        return Source::Unattributed;
    };
    if !trusted.contains(peer) {
        return Source::Address(key(peer));
    }
    let mut nearest = None;
    let mut examined = 0;
    for value in headers.get_all("x-forwarded-for").iter().rev() {
        let Ok(text) = value.to_str() else {
            return Source::Unattributed;
        };
        for hop in text.rsplit(',') {
            if examined == MAX_FORWARDED_HOPS {
                return Source::Unattributed;
            }
            examined += 1;
            match hop.trim().parse::<IpAddr>() {
                Ok(address) if trusted.contains(address) => {
                    nearest.get_or_insert(address);
                }
                Ok(address) => return Source::Address(key(address)),
                Err(_) => return Source::Unattributed,
            }
        }
    }
    nearest.map_or(Source::Unattributed, |address| {
        Source::Address(key(address))
    })
}

fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

fn key(address: IpAddr) -> SourceKey {
    match canonical(address) {
        IpAddr::V4(v4) => SourceKey(v4.to_ipv6_mapped().octets()),
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            SourceKey(octets)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn forwarded(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(
                "x-forwarded-for",
                HeaderValue::from_str(value).expect("header"),
            );
        }
        headers
    }

    fn address(text: &str) -> Source {
        Source::Address(key(text.parse().expect("address")))
    }

    #[test]
    fn an_untrusted_peer_is_the_source_whatever_it_forwards() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").expect("trusted");
        let headers = forwarded(&["198.51.100.7"]);
        assert_eq!(
            attribute(Some("203.0.113.9".parse().unwrap()), &headers, &trusted),
            address("203.0.113.9")
        );
        assert_eq!(
            attribute(
                Some("203.0.113.9".parse().unwrap()),
                &headers,
                &TrustedProxies::default()
            ),
            address("203.0.113.9")
        );
        let none = TrustedProxies::parse("").expect("an empty setting trusts nobody");
        assert!(none.is_empty());
        assert_eq!(
            attribute(Some("10.1.2.3".parse().unwrap()), &headers, &none),
            address("10.1.2.3")
        );
    }

    #[test]
    fn by_default_only_loopback_private_and_link_local_peers_may_forward() {
        let trusted = TrustedProxies::default();
        let headers = forwarded(&["198.51.100.7"]);
        for proxy in [
            "127.0.0.1",
            "::1",
            "10.4.0.9",
            "172.20.0.2",
            "192.168.1.1",
            "fd00::5",
        ] {
            assert_eq!(
                attribute(Some(proxy.parse().unwrap()), &headers, &trusted),
                address("198.51.100.7"),
                "{proxy}"
            );
        }
        for public in ["203.0.113.9", "2001:db8::1", "100.64.0.1"] {
            assert_eq!(
                attribute(Some(public.parse().unwrap()), &headers, &trusted),
                address(public),
                "{public}"
            );
        }
    }

    #[test]
    fn a_trusted_proxy_yields_the_nearest_untrusted_hop() {
        let trusted = TrustedProxies::parse("10.0.0.0/8, 192.0.2.10").expect("trusted");
        let peer = Some("10.1.2.3".parse().unwrap());
        assert_eq!(
            attribute(peer, &forwarded(&["198.51.100.7"]), &trusted),
            address("198.51.100.7")
        );
        assert_eq!(
            attribute(
                peer,
                &forwarded(&["203.0.113.1, 198.51.100.7, 192.0.2.10"]),
                &trusted
            ),
            address("198.51.100.7"),
            "a client-written leftmost hop is never believed"
        );
        assert_eq!(
            attribute(peer, &forwarded(&["203.0.113.1", "198.51.100.7"]), &trusted),
            address("198.51.100.7"),
            "repeated headers are read as one list in order"
        );
    }

    #[test]
    fn a_private_client_behind_a_private_proxy_is_its_own_source() {
        let trusted = TrustedProxies::default();
        let proxy = Some("172.20.0.2".parse().unwrap());
        assert_eq!(
            attribute(proxy, &forwarded(&["192.168.1.10"]), &trusted),
            address("192.168.1.10")
        );
        assert_ne!(
            attribute(proxy, &forwarded(&["192.168.1.11"]), &trusted),
            address("192.168.1.10")
        );
        assert_eq!(
            attribute(proxy, &forwarded(&["10.9.9.9, 192.168.1.10"]), &trusted),
            address("192.168.1.10"),
            "a client-written hop behind an appending proxy is never believed"
        );
    }

    #[test]
    fn a_trusted_proxy_without_a_readable_hop_is_unattributed() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").expect("trusted");
        let peer = Some("10.1.2.3".parse().unwrap());
        for headers in [
            HeaderMap::new(),
            forwarded(&["not-an-address"]),
            forwarded(&["198.51.100.7, garbage"]),
            forwarded(&["198.51.100.7:4431"]),
        ] {
            assert_eq!(attribute(peer, &headers, &trusted), Source::Unattributed);
        }
        assert_eq!(
            attribute(None, &forwarded(&["198.51.100.7"]), &trusted),
            Source::Unattributed
        );
    }

    #[test]
    fn forwarded_hops_beyond_the_bound_are_not_examined() {
        let trusted = TrustedProxies::parse("10.0.0.0/8").expect("trusted");
        let mut chain = vec!["198.51.100.7"];
        chain.extend(std::iter::repeat_n("10.0.0.1", MAX_FORWARDED_HOPS));
        let headers = forwarded(&[&chain.join(",")]);
        assert_eq!(
            attribute(Some("10.1.2.3".parse().unwrap()), &headers, &trusted),
            Source::Unattributed
        );
    }

    #[test]
    fn one_ipv6_slash_64_is_one_source_and_mapped_ipv4_is_ipv4() {
        assert_eq!(
            address("2001:db8:1:2:aaaa::1"),
            address("2001:db8:1:2:ffff:ffff:ffff:ffff")
        );
        assert_ne!(address("2001:db8:1:2::1"), address("2001:db8:1:3::1"));
        assert_eq!(address("::ffff:198.51.100.7"), address("198.51.100.7"));
        let trusted = TrustedProxies::parse("198.51.100.0/24").expect("trusted");
        assert!(trusted.contains("::ffff:198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn malformed_and_oversized_proxy_lists_are_refused_without_echoing_values() {
        let error = TrustedProxies::parse("10.0.0.0/8, secret-host").expect_err("refused");
        assert!(error.contains("entry 2"), "{error}");
        assert!(!error.contains("secret-host"), "{error}");
        let many = (0..=MAX_TRUSTED_PROXIES)
            .map(|index| format!("10.0.{index}.0/24"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(TrustedProxies::parse(&many).is_err());
        assert!(TrustedProxies::parse("  ").expect("empty").is_empty());
    }
}
