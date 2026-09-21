//! Trusted-proxy client-IP policy.
//!
//! `axum-client-addr` owns CIDR matching and trusted right-to-left resolution.
//! This module projects only enough of each supported header into canonical IP
//! chains to reject disagreement and bound work before delegating resolution.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName},
    middleware::Next,
    response::Response,
};
use axum_client_addr::{ChainHeader, ClientIpConfig, ClientIpSource, IpCidr};

const MAX_CHAIN_HOPS: usize = 32;
const FORWARDED: HeaderName = HeaderName::from_static("forwarded");
const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// An immutable set of proxy networks trusted to supply forwarding evidence.
#[derive(Clone, Debug)]
pub struct TrustedProxyConfig {
    forwarded_resolver: ClientIpConfig,
    x_forwarded_for_resolver: ClientIpConfig,
}

impl TrustedProxyConfig {
    /// Construct the default configuration that trusts no proxy.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            forwarded_resolver: ClientIpConfig::builder().trust_no_proxy(),
            x_forwarded_for_resolver: ClientIpConfig::builder().trust_no_proxy(),
        }
    }

    /// Construct a configuration from the trusted proxy network set.
    ///
    /// An empty set trusts no peer, so forwarding headers remain inert.
    ///
    /// # Errors
    ///
    /// Returns an error when the dependency cannot construct an unambiguous
    /// trusted-proxy rule set from the supplied networks.
    pub fn new(proxies: impl IntoIterator<Item = IpCidr>) -> Result<Self, TrustedProxyConfigError> {
        let proxies = proxies.into_iter().collect::<Vec<_>>();
        Ok(Self {
            forwarded_resolver: resolver_for(&proxies, FORWARDED)?,
            x_forwarded_for_resolver: resolver_for(&proxies, X_FORWARDED_FOR)?,
        })
    }

    /// Resolve one request without replacing its direct Transport Peer.
    #[must_use]
    pub fn resolve(
        &self,
        headers: &HeaderMap,
        transport_peer: Option<SocketAddr>,
    ) -> RequestAddress {
        let Some(transport_peer) = transport_peer else {
            return RequestAddress {
                transport_peer: None,
                effective_client_ip: None,
                outcome: ResolutionOutcome::TransportUnavailable,
            };
        };
        let peer_ip = canonical_ip(transport_peer.ip());
        if !self.x_forwarded_for_resolver.is_trusted_proxy(peer_ip) {
            return RequestAddress::socket(transport_peer, peer_ip);
        }

        let forwarded = canonical_chain(headers, &FORWARDED, HeaderFamily::Forwarded);
        let x_forwarded_for =
            canonical_chain(headers, &X_FORWARDED_FOR, HeaderFamily::XForwardedFor);
        // Matching `Forwarded` first is safe after agreement: both chains then
        // name the same hops, and the dependency still performs trust traversal.
        let selected_resolver = if matches!(&forwarded, Ok(Some(_))) {
            &self.forwarded_resolver
        } else {
            &self.x_forwarded_for_resolver
        };
        let outcome = match (forwarded, x_forwarded_for) {
            (Err(ChainError::OverLimit), _) | (_, Err(ChainError::OverLimit)) => {
                ResolutionOutcome::OverLimit
            }
            (Err(ChainError::Malformed), _) | (_, Err(ChainError::Malformed)) => {
                ResolutionOutcome::Malformed
            }
            (Ok(Some(left)), Ok(Some(right))) if left != right => ResolutionOutcome::Conflict,
            (Ok(Some(_) | None), Ok(Some(_))) | (Ok(Some(_)), Ok(None)) => {
                let client_ip = selected_resolver.resolve_client_ip(headers, peer_ip);
                if matches!(client_ip.source(), ClientIpSource::ChainHeader(_)) {
                    return RequestAddress {
                        transport_peer: Some(transport_peer),
                        effective_client_ip: Some(client_ip.ip()),
                        outcome: ResolutionOutcome::Forwarded,
                    };
                }
                ResolutionOutcome::Socket
            }
            (Ok(None), Ok(None)) => ResolutionOutcome::Socket,
        };

        RequestAddress {
            transport_peer: Some(transport_peer),
            effective_client_ip: Some(peer_ip),
            outcome,
        }
    }
}

impl Default for TrustedProxyConfig {
    fn default() -> Self {
        Self::empty()
    }
}

fn resolver_for(
    proxies: &[IpCidr],
    header: HeaderName,
) -> Result<ClientIpConfig, axum_client_addr::ClientIpConfigBuildError> {
    proxies
        .iter()
        .copied()
        .fold(
            ClientIpConfig::builder().trusted_proxies(),
            axum_client_addr::TrustedProxiesBuilder::proxy,
        )
        .chain_header_order([ChainHeader::new(header)])
        .build()
}

/// Why a trusted-proxy configuration could not be constructed.
#[derive(Debug, thiserror::Error)]
pub enum TrustedProxyConfigError {
    /// The dependency rejected conflicting trusted-proxy configuration.
    #[error(transparent)]
    Resolver(#[from] axum_client_addr::ClientIpConfigBuildError),
}

/// The request-local addresses retained by trusted-proxy resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestAddress {
    /// The unchanged direct socket endpoint.
    pub transport_peer: Option<SocketAddr>,
    /// The derived IP-only client attribution, if a Transport Peer was available.
    pub effective_client_ip: Option<IpAddr>,
    /// The bounded reason that selected the effective address.
    pub outcome: ResolutionOutcome,
}

impl RequestAddress {
    const fn socket(transport_peer: SocketAddr, effective_client_ip: IpAddr) -> Self {
        Self {
            transport_peer: Some(transport_peer),
            effective_client_ip: Some(effective_client_ip),
            outcome: ResolutionOutcome::Socket,
        }
    }
}

/// A bounded trusted-proxy resolution result suitable for telemetry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionOutcome {
    /// No forwarding evidence changed the Transport Peer's IP.
    Socket,
    /// A trusted, valid forwarding chain supplied the Effective Client IP.
    Forwarded,
    /// A supplied forwarding field could not form one accepted IP chain.
    Malformed,
    /// The two supplied forwarding families described different IP chains.
    Conflict,
    /// A supplied forwarding chain exceeded the fixed work bound.
    OverLimit,
    /// No direct socket endpoint was available to root forwarding trust.
    TransportUnavailable,
}

impl ResolutionOutcome {
    /// The closed telemetry value for this resolution outcome.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Socket => "socket",
            Self::Forwarded => "forwarded",
            Self::Malformed => "malformed",
            Self::Conflict => "conflict",
            Self::OverLimit => "over-limit",
            Self::TransportUnavailable => "transport-unavailable",
        }
    }
}

/// Resolves and attaches request-local address context without replacing Axum's
/// Transport Peer extension.
async fn resolve_request_address(
    State(config): State<TrustedProxyConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    let transport_peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer);
    let address = config.resolve(request.headers(), transport_peer);
    request.extensions_mut().insert(address);
    next.run(request).await
}

/// Adds trusted-proxy address context before the HTTP observability layer.
pub fn with_request_address<S>(router: Router<S>, config: TrustedProxyConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(axum::middleware::from_fn_with_state(
        config,
        resolve_request_address,
    ))
}

#[derive(Clone, Copy)]
enum HeaderFamily {
    Forwarded,
    XForwardedFor,
}

#[derive(Clone, Copy, Debug)]
enum ChainError {
    Malformed,
    OverLimit,
}

fn canonical_chain(
    headers: &HeaderMap,
    name: &HeaderName,
    family: HeaderFamily,
) -> Result<Option<Vec<IpAddr>>, ChainError> {
    let values = headers.get_all(name);
    if values.iter().next().is_none() {
        return Ok(None);
    }

    let mut chain = Vec::new();
    for value in values {
        let value = value.to_str().map_err(|_| ChainError::Malformed)?;
        let entries = match family {
            HeaderFamily::Forwarded => parse_forwarded(value)?,
            HeaderFamily::XForwardedFor => parse_x_forwarded_for(value)?,
        };
        chain.extend(entries);
        if chain.len() > MAX_CHAIN_HOPS {
            return Err(ChainError::OverLimit);
        }
    }
    Ok(Some(chain))
}

fn parse_x_forwarded_for(value: &str) -> Result<Vec<IpAddr>, ChainError> {
    value
        .split(',')
        .map(|node| parse_node(node.trim()))
        .collect()
}

fn parse_forwarded(value: &str) -> Result<Vec<IpAddr>, ChainError> {
    split_quoted(value, ',')?
        .into_iter()
        .map(|element| {
            let mut node = None;
            for parameter in split_quoted(element, ';')? {
                let (name, value) = parameter.split_once('=').ok_or(ChainError::Malformed)?;
                if name.trim().eq_ignore_ascii_case("for")
                    && node.replace(unquote(value.trim())?).is_some()
                {
                    return Err(ChainError::Malformed);
                }
            }
            parse_node(node.ok_or(ChainError::Malformed)?)
        })
        .collect()
}

/// Split an HTTP list without treating delimiters in a quoted string as hops.
fn split_quoted(value: &str, delimiter: char) -> Result<Vec<&str>, ChainError> {
    let mut values = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == delimiter && !quoted {
            let entry = value[start..offset].trim();
            if entry.is_empty() {
                return Err(ChainError::Malformed);
            }
            values.push(entry);
            start = offset + character.len_utf8();
        }
    }
    if quoted || escaped {
        return Err(ChainError::Malformed);
    }
    let entry = value[start..].trim();
    if entry.is_empty() {
        return Err(ChainError::Malformed);
    }
    values.push(entry);
    Ok(values)
}

fn unquote(value: &str) -> Result<&str, ChainError> {
    if !value.starts_with('"') && !value.ends_with('"') {
        return Ok(value);
    }
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(ChainError::Malformed);
    }
    // Escaped node values are not accepted: they cannot be an unambiguous IP node.
    let value = &value[1..value.len() - 1];
    if value.contains('\\') {
        return Err(ChainError::Malformed);
    }
    Ok(value)
}

fn parse_node(value: &str) -> Result<IpAddr, ChainError> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("unknown")
        || value.starts_with('_')
        || value.contains('%')
    {
        return Err(ChainError::Malformed);
    }
    if let Ok(ip) = value.parse() {
        return Ok(canonical_ip(ip));
    }
    if let Some(value) = value.strip_prefix('[') {
        let Some((ip, port)) = value.split_once(']') else {
            return Err(ChainError::Malformed);
        };
        if let Some(port) = port.strip_prefix(':') {
            if !is_valid_node_port(port) {
                return Err(ChainError::Malformed);
            }
        } else if !port.is_empty() {
            return Err(ChainError::Malformed);
        }
        return ip
            .parse()
            .map(canonical_ip)
            .map_err(|_| ChainError::Malformed);
    }
    let Some((host, port)) = value.rsplit_once(':') else {
        return Err(ChainError::Malformed);
    };
    if host.parse::<Ipv4Addr>().is_ok() && is_valid_node_port(port) {
        return host
            .parse()
            .map(canonical_ip)
            .map_err(|_| ChainError::Malformed);
    }
    Err(ChainError::Malformed)
}

fn is_valid_node_port(port: &str) -> bool {
    (!port.is_empty() && port.chars().all(|character| character.is_ascii_digit()))
        || port.strip_prefix('_').is_some_and(|obfuscated| {
            !obfuscated.is_empty()
                && obfuscated.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
                })
        })
}

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        IpAddr::V4(ip) => IpAddr::V4(ip),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, SocketAddr};

    use axum::{
        Router,
        body::Body,
        extract::{ConnectInfo, Request},
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    use axum_client_addr::IpCidr;
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{RequestAddress, ResolutionOutcome, TrustedProxyConfig};

    fn config(proxies: &[&str]) -> TrustedProxyConfig {
        TrustedProxyConfig::new(proxies.iter().map(|proxy| proxy.parse::<IpCidr>().unwrap()))
            .unwrap()
    }

    fn peer(value: &str) -> SocketAddr {
        value.parse().unwrap()
    }

    fn headers(values: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in values {
            headers.append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    fn address(
        config: &TrustedProxyConfig,
        values: &[(&str, &str)],
        transport_peer: Option<&str>,
    ) -> RequestAddress {
        config.resolve(&headers(values), transport_peer.map(peer))
    }

    #[test]
    fn default_configuration_ignores_spoofed_forwarding_headers() {
        let resolved = address(
            &config(&[]),
            &[
                ("forwarded", "for=203.0.113.10"),
                ("x-forwarded-for", "203.0.113.10"),
            ],
            Some("192.0.2.20:443"),
        );
        assert_eq!(resolved.transport_peer, Some(peer("192.0.2.20:443")));
        assert_eq!(
            resolved.effective_client_ip,
            Some("192.0.2.20".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Socket);
    }

    #[test]
    fn configured_but_untrusted_peer_ignores_spoofed_forwarding_headers() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[("x-forwarded-for", "203.0.113.10")],
            Some("192.0.2.20:443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("192.0.2.20".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Socket);
    }

    #[test]
    fn absent_forwarding_headers_leave_the_effective_client_at_the_transport_peer() {
        let resolved = address(&config(&["10.0.0.0/24"]), &[], Some("10.0.0.2:8443"));
        assert_eq!(resolved.transport_peer, Some(peer("10.0.0.2:8443")));
        assert_eq!(
            resolved.effective_client_ip,
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Socket);
    }

    #[test]
    fn configured_single_proxy_derives_effective_client_and_preserves_peer() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[("x-forwarded-for", "203.0.113.10")],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(resolved.transport_peer, Some(peer("10.0.0.2:8443")));
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn configured_single_proxy_accepts_rfc_forwarded() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[("forwarded", "for=203.0.113.10")],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn trusted_chain_stops_at_first_untrusted_hop() {
        let resolved = address(
            &config(&["10.0.0.0/24", "198.51.100.0/24"]),
            &[("x-forwarded-for", "203.0.113.10, 198.51.100.7")],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn all_trusted_hops_fall_back_to_transport_peer() {
        let resolved = address(
            &config(&["10.0.0.0/24", "198.51.100.0/24"]),
            &[("x-forwarded-for", "198.51.100.7")],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Socket);
    }

    #[test]
    fn bare_proxy_address_and_ipv4_ports_are_accepted() {
        let resolved = address(
            &config(&["10.0.0.2"]),
            &[
                ("forwarded", "for=203.0.113.10:443"),
                ("x-forwarded-for", "203.0.113.10:443"),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn resolution_outcome_has_only_the_documented_telemetry_values() {
        assert_eq!(ResolutionOutcome::Socket.as_str(), "socket");
        assert_eq!(ResolutionOutcome::Forwarded.as_str(), "forwarded");
        assert_eq!(ResolutionOutcome::Malformed.as_str(), "malformed");
        assert_eq!(ResolutionOutcome::Conflict.as_str(), "conflict");
        assert_eq!(ResolutionOutcome::OverLimit.as_str(), "over-limit");
        assert_eq!(
            ResolutionOutcome::TransportUnavailable.as_str(),
            "transport-unavailable"
        );
    }

    #[test]
    fn canonical_node_adapter_accepts_obfuscated_ports() {
        assert!(
            super::parse_node("[2001:db8::1]:_edge.1")
                .is_ok_and(|ip| ip == "2001:db8::1".parse::<IpAddr>().unwrap())
        );
        assert!(
            super::parse_node("203.0.113.10:_edge-1")
                .is_ok_and(|ip| ip == "203.0.113.10".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn canonical_chain_parser_handles_quoted_delimiters_and_rejects_invalid_syntax() {
        assert_eq!(
            super::split_quoted("for=\"203.0.113.10\\\\quoted\";by=proxy", ';')
                .expect("quoted delimiter"),
            ["for=\"203.0.113.10\\\\quoted\"", "by=proxy"]
        );
        for value in [
            "",
            "for=203.0.113.10,",
            ",for=203.0.113.10",
            "for=\"unterminated",
            "for=\"trailing\\",
        ] {
            assert!(super::split_quoted(value, ',').is_err(), "{value:?}");
        }
        for value in ["\"only-open", "only-close\"", "\"contains\\\\escape\""] {
            assert!(super::unquote(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn canonical_node_adapter_rejects_invalid_bracketed_tails() {
        for value in ["[2001:db8::1]unexpected", "[2001:db8::1]:"] {
            assert!(super::parse_node(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn matching_forwarded_families_accept_quoted_ipv6_numeric_port() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[
                ("forwarded", "for=\"[2001:db8::1]:443\""),
                ("x-forwarded-for", "[2001:db8::1]:443"),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("2001:db8::1".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn matching_forwarded_families_accept_quoted_obfuscated_ipv4_port() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[
                ("forwarded", "for=\"203.0.113.10:_edge-1\""),
                ("x-forwarded-for", "203.0.113.10:_edge-1"),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[rstest]
    #[case("for=unknown", "203.0.113.10")]
    #[case("for=_hidden", "203.0.113.10")]
    #[case("for=example.test", "203.0.113.10")]
    #[case("for=\"203.0.113.10:_\"", "203.0.113.10")]
    #[case("for=\"[2001:db8::1]:_invalid!\"", "2001:db8::1")]
    #[case("for=\"[2001:db8::1\"", "203.0.113.10")]
    #[case("for=203.0.113.10", "203.0.113.10,")]
    fn malformed_forwarding_evidence_falls_back_to_peer(
        #[case] forwarded: &str,
        #[case] x_forwarded_for: &str,
    ) {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[
                ("forwarded", forwarded),
                ("x-forwarded-for", x_forwarded_for),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Malformed);
    }

    #[test]
    fn conflicting_header_families_fall_back_to_peer() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[
                ("forwarded", "for=203.0.113.10"),
                ("x-forwarded-for", "203.0.113.11"),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Conflict);
    }

    #[test]
    fn repeated_header_lines_form_one_ordered_chain() {
        let resolved = address(
            &config(&["10.0.0.0/24", "198.51.100.0/24"]),
            &[
                ("x-forwarded-for", "203.0.113.10"),
                ("x-forwarded-for", "198.51.100.7"),
            ],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn over_limit_forwarding_evidence_falls_back_to_peer() {
        let chain = std::iter::repeat_n("203.0.113.10", 33)
            .collect::<Vec<_>>()
            .join(", ");
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[("x-forwarded-for", &chain)],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("10.0.0.2".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::OverLimit);
    }

    #[test]
    fn mapped_ipv6_forms_match_ipv4_cidrs_and_compare_equally() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[
                ("forwarded", "for=::ffff:203.0.113.10"),
                ("x-forwarded-for", "203.0.113.10"),
            ],
            Some("[::ffff:10.0.0.2]:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some(IpAddr::V4("203.0.113.10".parse().unwrap()))
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[test]
    fn duplicate_and_overlapping_proxy_ranges_remain_one_trust_set() {
        let resolved = address(
            &config(&["10.0.0.0/8", "10.0.0.0/24", "10.0.0.2"]),
            &[("x-forwarded-for", "203.0.113.10")],
            Some("10.0.0.2:8443"),
        );
        assert_eq!(
            resolved.effective_client_ip,
            Some("203.0.113.10".parse().unwrap())
        );
        assert_eq!(resolved.outcome, ResolutionOutcome::Forwarded);
    }

    #[tokio::test]
    async fn middleware_preserves_transport_peer_and_inserts_request_address() {
        async fn context(request: Request) -> String {
            let peer = request.extensions().get::<ConnectInfo<SocketAddr>>();
            let address = request.extensions().get::<RequestAddress>();
            (peer.is_some_and(|ConnectInfo(peer)| *peer == "10.0.0.2:443".parse().unwrap())
                && address.is_some_and(|address| {
                    address.transport_peer == Some("10.0.0.2:443".parse().unwrap())
                        && address.effective_client_ip == Some("203.0.113.10".parse().unwrap())
                }))
            .to_string()
        }

        let app = super::with_request_address(
            Router::new().route("/", get(context)),
            config(&["10.0.0.0/24"]),
        );
        let mut request = axum::http::Request::builder()
            .uri("/")
            .header("x-forwarded-for", "203.0.113.10")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer("10.0.0.2:443")));

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("context response"),
            "true"
        );
    }

    #[test]
    fn missing_transport_peer_exposes_no_effective_client_ip() {
        let resolved = address(
            &config(&["10.0.0.0/24"]),
            &[("x-forwarded-for", "203.0.113.10")],
            None,
        );
        assert_eq!(resolved.transport_peer, None);
        assert_eq!(resolved.effective_client_ip, None);
        assert_eq!(resolved.outcome, ResolutionOutcome::TransportUnavailable);
    }
}
