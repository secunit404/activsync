use std::net::IpAddr;
use std::sync::Mutex;

use axum::http::{HeaderMap, HeaderValue};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Network {
    address: IpAddr,
    pub(crate) prefix: u8,
}

impl Network {
    /// Parses "10.0.0.0/8" or a bare address. Host bits are ignored, as with
    /// Python's `ip_network(strict=False)`.
    pub fn parse(text: &str) -> Option<Self> {
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = addr.trim().parse().ok()?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.trim().parse::<u8>().ok().filter(|p| *p <= max)?,
            None => max,
        };
        Some(Self { address, prefix })
    }

    /// Python's `ip_network(strict=True)`: host bits must be zero, the prefix is
    /// digits only, and IPv4 also accepts a netmask or hostmask.
    fn parse_strict(text: &str) -> Option<Self> {
        let (addr, prefix) = text.split_once('/')?;
        let address: IpAddr = addr.parse().ok()?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let digits_only = !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit());
        let prefix = if digits_only {
            prefix.parse::<u32>().ok().filter(|p| *p <= max)? as u8
        } else if address.is_ipv4() {
            let mask = u32::from(prefix.parse::<std::net::Ipv4Addr>().ok()?);
            [mask, !mask]
                .into_iter()
                .find(|m| m.leading_ones() + m.trailing_zeros() == 32)
                .map(|m| m.leading_ones() as u8)?
        } else {
            return None;
        };
        let network = Self { address, prefix };
        (network.masked() == address).then_some(network)
    }

    fn masked(&self) -> IpAddr {
        match self.address {
            IpAddr::V4(a) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(self.prefix))
                };
                IpAddr::V4((u32::from(a) & mask).into())
            }
            IpAddr::V6(a) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(self.prefix))
                };
                IpAddr::V6((u128::from(a) & mask).into())
            }
        }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(self.prefix))
                };
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(self.prefix))
                };
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.masked(), self.prefix)
    }
}

/// Who may set X-Forwarded-For/-Proto, from `FORWARDED_ALLOW_IPS`, parsed as
/// uvicorn's `ProxyHeadersMiddleware` does: an entry with a `/` is a strict
/// network, a bare IP an address, and anything that fails to parse (including
/// a network with host bits set) is a literal matched against the peer text.
#[derive(Clone, Debug)]
pub struct ForwardedTrust {
    always: bool,
    networks: Vec<Network>,
    literals: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Forwarded {
    pub client: Option<(String, i64)>,
    pub scheme: Option<String>,
}

pub fn latin1(value: &HeaderValue) -> String {
    value.as_bytes().iter().map(|b| char::from(*b)).collect()
}

/// Python's `int(text)` for the digits-and-sign forms a port can take.
fn py_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let well_formed = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.chars().all(|c| c.is_ascii_digit() || c == '_');
    let value: i64 = well_formed
        .then(|| digits.replace('_', "").parse().ok())
        .flatten()?;
    Some(if negative { -value } else { value })
}

/// uvicorn's `_parse_host_port`: bare IPs, `host:port`, and `[v6]:port`;
/// anything malformed comes back whole with port 0.
fn parse_host_port(value: &str) -> (String, i64) {
    if let Some(rest) = value.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return (value.to_string(), 0);
        };
        let (host, remainder) = (&rest[..end], &rest[end + 1..]);
        if remainder.is_empty() {
            return (host.to_string(), 0);
        }
        let Some(port) = remainder.strip_prefix(':') else {
            return (value.to_string(), 0);
        };
        return (host.to_string(), py_int(port).unwrap_or(0));
    }
    if value.matches(':').count() == 1
        && let Some((host, port)) = value.rsplit_once(':')
    {
        return match py_int(port) {
            Some(port) => (host.to_string(), port),
            None => (value.to_string(), 0),
        };
    }
    (value.to_string(), 0)
}

impl ForwardedTrust {
    pub fn parse(value: &str) -> Self {
        let mut trust = Self {
            always: value == "*",
            networks: Vec::new(),
            literals: Vec::new(),
        };
        if trust.always {
            return trust;
        }
        for item in value.split(',').map(str::trim) {
            let network = if item.contains('/') {
                Network::parse_strict(item)
            } else {
                item.parse::<IpAddr>().ok().map(|address| Network {
                    address,
                    prefix: if address.is_ipv4() { 32 } else { 128 },
                })
            };
            match network {
                Some(network) => trust.networks.push(network),
                None => trust.literals.push(item.to_string()),
            }
        }
        trust
    }

    pub fn trusts(&self, host: Option<&str>) -> bool {
        if self.always {
            return true;
        }
        let Some(host) = host.filter(|h| !h.is_empty()) else {
            return false;
        };
        match host.parse::<IpAddr>() {
            Ok(ip) => self.networks.iter().any(|n| n.contains(ip)),
            Err(_) => self.literals.iter().any(|l| l == host),
        }
    }

    /// uvicorn's rule: the rightmost forwarded address that is not itself a
    /// trusted proxy is the client; if all are trusted, the leftmost.
    fn client_from(&self, forwarded_for: &str) -> (String, i64) {
        let hosts: Vec<&str> = forwarded_for.split(',').map(str::trim).collect();
        if self.always {
            return parse_host_port(hosts[0]);
        }
        hosts
            .iter()
            .rev()
            .map(|h| parse_host_port(h))
            .find(|(host, _)| !self.trusts(Some(host)))
            .unwrap_or_else(|| parse_host_port(hosts[0]))
    }

    /// What uvicorn's middleware would rewrite for a request from `peer`: every
    /// X-Forwarded-For line joined with ", ", the last X-Forwarded-Proto line,
    /// the proto taken only when it is exactly http, https, ws or wss, and an
    /// empty client leaving the socket peer in place.
    pub fn apply(&self, peer: Option<&str>, headers: &HeaderMap) -> Forwarded {
        if !self.trusts(peer) {
            return Forwarded::default();
        }
        let scheme = headers
            .get_all("x-forwarded-proto")
            .iter()
            .next_back()
            .map(|v| latin1(v).trim().to_string())
            .filter(|p| matches!(p.as_str(), "http" | "https" | "ws" | "wss"));
        let lines: Vec<String> = headers
            .get_all("x-forwarded-for")
            .iter()
            .map(latin1)
            .collect();
        let client = (!lines.is_empty())
            .then(|| self.client_from(&lines.join(", ")))
            .filter(|(host, _)| !host.is_empty());
        Forwarded { client, scheme }
    }
}

/// Python's `repr()` of a string made of Latin-1 characters.
pub fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (0x7f..=0xa0).contains(&(c as u32)) || c as u32 == 0xad => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Each warning at most once a minute, so a scanner or a misrouted proxy
/// cannot fill the log.
#[derive(Debug, Default)]
pub struct WarnLimits {
    unknown_host: Mutex<Option<f64>>,
    untrusted_proxy: Mutex<Option<f64>>,
}

impl WarnLimits {
    fn due(slot: &Mutex<Option<f64>>, now: f64) -> bool {
        let mut last = slot.lock().unwrap_or_else(|p| p.into_inner());
        if last.is_some_and(|at| now - at < 60.0) {
            return false;
        }
        *last = Some(now);
        true
    }

    pub fn unknown_host_due(&self, now: f64) -> bool {
        Self::due(&self.unknown_host, now)
    }

    pub fn untrusted_proxy_due(&self, now: f64) -> bool {
        Self::due(&self.untrusted_proxy, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_trust_parses_like_uvicorn() {
        let cases: &[(&str, &[(&str, bool)])] = &[
            (
                "127.0.0.1",
                &[
                    ("127.0.0.1", true),
                    ("127.0.0.2", false),
                    ("::1", false),
                    ("", false),
                ],
            ),
            (
                "172.18.0.5/16",
                &[
                    ("172.18.0.5", false),
                    ("172.18.9.9", false),
                    ("172.18.0.5/16", true),
                ],
            ),
            (
                "172.18.0.0/16",
                &[
                    ("172.18.9.9", true),
                    ("172.19.0.1", false),
                    ("::ffff:172.18.0.1", false),
                ],
            ),
            (
                "10.0.0.0/255.0.0.0",
                &[("10.1.2.3", true), ("11.0.0.1", false)],
            ),
            (
                "10.0.0.0/0.255.255.255",
                &[("10.1.2.3", true), ("11.0.0.1", false)],
            ),
            ("10.0.0.0/+8", &[("10.1.2.3", false), ("10.0.0.0/+8", true)]),
            ("10.0.0.0/08", &[("10.1.2.3", true)]),
            (
                "10.0.0.1/255.0.0.0",
                &[("10.0.0.1", false), ("10.0.0.1/255.0.0.0", true)],
            ),
            ("*", &[("1.2.3.4", true), ("anything", true), ("", true)]),
            (" *", &[("1.2.3.4", false), ("*", true)]),
            ("127.0.0.1, *", &[("1.2.3.4", false), ("*", true)]),
            (
                "::1",
                &[
                    ("::1", true),
                    ("0:0:0:0:0:0:0:1", true),
                    ("127.0.0.1", false),
                ],
            ),
            ("fd00::/8", &[("fd12::1", true), ("fe80::1", false)]),
            ("fd00::1/8", &[("fd12::1", false), ("fd00::1/8", true)]),
            ("unix:", &[("unix:", true), ("1.2.3.4", false)]),
            ("127.0.0.1, ::1/128", &[("127.0.0.1", true), ("::1", true)]),
            ("", &[("", false), ("1.2.3.4", false)]),
            ("1.2.3.4,,5.6.7.8", &[("5.6.7.8", true), ("", false)]),
            ("10.0.0.0/33", &[("10.0.0.1", false), ("10.0.0.0/33", true)]),
            (
                "10.0.0.0/8/9",
                &[("10.0.0.1", false), ("10.0.0.0/8/9", true)],
            ),
        ];
        for (allow, hosts) in cases {
            let trust = ForwardedTrust::parse(allow);
            for (host, expected) in *hosts {
                assert_eq!(
                    trust.trusts(Some(host)),
                    *expected,
                    "{allow:?} trusts {host:?}"
                );
            }
        }
    }

    fn lines(name: &'static str, values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn forwarded_for_matches_uvicorn() {
        type Case<'a> = (&'a str, &'a str, &'a [&'a str], Option<(&'a str, i64)>);
        let cases: &[Case] = &[
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["203.0.113.9"],
                Some(("203.0.113.9", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["203.0.113.9:4444"],
                Some(("203.0.113.9", 4444)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.1.1.1", "203.0.113.9, 10.0.0.2"],
                Some(("203.0.113.9", 0)),
            ),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &[""], None),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["[2001:db8::1]:5000"],
                Some(("2001:db8::1", 5000)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["2001:db8::1"],
                Some(("2001:db8::1", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["10.0.0.1, 127.0.0.1"],
                Some(("10.0.0.1", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["garbage:abc"],
                Some(("garbage:abc", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.2.3.4:80:90"],
                Some(("1.2.3.4:80:90", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.2.3.4, , 5.6.7.8"],
                Some(("5.6.7.8", 0)),
            ),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &["1.2.3.4,"], None),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.2.3.4:+80"],
                Some(("1.2.3.4", 80)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.2.3.4: 80"],
                Some(("1.2.3.4", 80)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["1.2.3.4:8_0"],
                Some(("1.2.3.4", 80)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["[::1]:x"],
                Some(("::1", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["[::1]x"],
                Some(("[::1]x", 0)),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["[::1"],
                Some(("[::1", 0)),
            ),
            ("127.0.0.1, 10.0.0.0/8", "8.8.8.8", &["203.0.113.9"], None),
            ("*", "9.9.9.9", &["1.1.1.1, 2.2.2.2"], Some(("1.1.1.1", 0))),
            ("172.18.0.5/16", "172.18.0.5", &["203.0.113.9"], None),
        ];
        for (allow, peer, header_lines, expected) in cases {
            let applied = ForwardedTrust::parse(allow)
                .apply(Some(peer), &lines("x-forwarded-for", header_lines));
            let expected = expected.map(|(host, port)| (host.to_string(), port));
            assert_eq!(
                applied.client, expected,
                "{allow:?} from {peer} sending {header_lines:?}"
            );
        }
    }

    #[test]
    fn forwarded_proto_matches_uvicorn() {
        let cases: &[(&str, &str, &[&str], Option<&str>)] = &[
            ("127.0.0.1, 10.0.0.0/8", "8.8.8.8", &["https"], None),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["https"],
                Some("https"),
            ),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &[" HTTPS "], None),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &[" https "],
                Some("https"),
            ),
            (
                "127.0.0.1, 10.0.0.0/8",
                "127.0.0.1",
                &["https", "http"],
                Some("http"),
            ),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &["wss"], Some("wss")),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &["ws"], Some("ws")),
            ("127.0.0.1, 10.0.0.0/8", "127.0.0.1", &["ftp"], None),
            ("*", "9.9.9.9", &["https"], Some("https")),
        ];
        for (allow, peer, header_lines, expected) in cases {
            let applied = ForwardedTrust::parse(allow)
                .apply(Some(peer), &lines("x-forwarded-proto", header_lines));
            assert_eq!(
                applied.scheme.as_deref(),
                *expected,
                "{allow:?} from {peer} sending {header_lines:?}"
            );
        }
    }

    #[test]
    fn warnings_come_once_a_minute() {
        let limits = WarnLimits::default();
        assert!(limits.unknown_host_due(5.0));
        assert!(!limits.unknown_host_due(64.9));
        assert!(
            limits.untrusted_proxy_due(10.0),
            "each warning has its own clock"
        );
        assert!(limits.unknown_host_due(65.0));
    }

    #[test]
    fn python_repr_of_header_text() {
        assert_eq!(py_repr("evil.example"), "'evil.example'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a\"b'c"), "'a\"b\\'c'");
        assert_eq!(py_repr("caf\u{e9}"), "'caf\u{e9}'");
        assert_eq!(
            py_repr("a\u{80}b\u{a0}c\u{ad}d\x01\\"),
            "'a\\x80b\\xa0c\\xadd\\x01\\\\'"
        );
    }
}
