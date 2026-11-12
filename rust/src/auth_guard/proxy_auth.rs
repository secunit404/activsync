use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::is_on;
use crate::proxy_headers::Network;

const TRUST_GATEWAY_VARIABLE: &str = "ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY";

/// Sign-in through an authenticating reverse proxy.
#[derive(Clone, Debug)]
pub struct ProxyAuth {
    pub header: String,
    pub trusted: Vec<Network>,
    pub user: String,
    pub trusted_gateways: Vec<IpAddr>,
}

impl ProxyAuth {
    /// Refuses a trusted range that holds the container's default gateway:
    /// Docker relays outside connections from that address, so anyone who can
    /// reach the port could send the header and be signed in as the owner.
    pub fn from_env(
        get: impl Fn(&str) -> Option<String>,
        gateways: &[IpAddr],
    ) -> Result<Option<Self>, String> {
        let var = |name: &str| get(name).map(|v| v.trim().to_string()).unwrap_or_default();
        let header = var("ACTIVSYNC_AUTH_PROXY_HEADER");
        if header.is_empty() {
            return Ok(None);
        }
        let trusted_raw = var("ACTIVSYNC_AUTH_PROXY_TRUSTED");
        let user = var("ACTIVSYNC_AUTH_PROXY_USER");
        if trusted_raw.is_empty() {
            return Err("ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_TRUSTED must list the proxy's address range (for example 172.18.0.0/16).".into());
        }
        if user.is_empty() {
            return Err("ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_USER must name the user the proxy signs in as.".into());
        }
        let trusted = trusted_raw
            .split(',')
            .map(str::trim)
            .filter(|i| !i.is_empty())
            .map(|item| match Network::parse(item) {
                None => Err(format!(
                    "ACTIVSYNC_AUTH_PROXY_TRUSTED has an invalid address range: {item}"
                )),
                Some(network) if network.prefix == 0 => Err(format!(
                    "ACTIVSYNC_AUTH_PROXY_TRUSTED must not trust every address ({item})."
                )),
                Some(network) => Ok(network),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let in_range: Vec<IpAddr> = gateways
            .iter()
            .copied()
            .filter(|g| trusted.iter().any(|n| n.contains(*g)))
            .collect();
        let trust_gateway = is_on(&var(TRUST_GATEWAY_VARIABLE));
        if !in_range.is_empty() && !trust_gateway {
            let listed: Vec<String> = in_range.iter().map(IpAddr::to_string).collect();
            return Err(format!(
                "ACTIVSYNC_AUTH_PROXY_TRUSTED must not include this container's default gateway ({}). \
                 Docker's gateway relays connections that come from outside the Docker network, so anyone \
                 who can reach the published port could send the proxy header and be signed in as you. \
                 Trust only the reverse proxy's own address: give it a fixed IP and trust its /32. If the \
                 proxy runs on the Docker host itself, set {TRUST_GATEWAY_VARIABLE}=1 and keep the port \
                 reachable from that host alone.",
                listed.join(", ")
            ));
        }
        Ok(Some(Self {
            header,
            trusted,
            user,
            trusted_gateways: if trust_gateway { in_range } else { Vec::new() },
        }))
    }

    pub fn trusts(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| self.trusted.iter().any(|n| n.contains(ip)))
    }
}

/// Gateways of default routes from the Linux routing tables; empty where
/// they do not exist, such as on a Mac.
pub(super) fn read_default_gateways() -> Vec<IpAddr> {
    const ANY_V4: &str = "00000000";
    const ANY_V6: &str = "00000000000000000000000000000000";
    let table = |path| std::fs::read_to_string(path).unwrap_or_default();
    let (v4, v6) = (table("/proc/net/route"), table("/proc/net/ipv6_route"));
    let v4 = v4.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [_, ANY_V4, gateway, ..] = fields.as_slice() else {
            return None;
        };
        let word = u32::from_str_radix(gateway, 16).ok()?;
        Some(IpAddr::V4(Ipv4Addr::from(word.to_le_bytes())))
    });
    let v6 = v6.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [ANY_V6, "00", _, _, gateway, ..] = fields.as_slice() else {
            return None;
        };
        let value = u128::from_str_radix(gateway, 16).ok()?;
        Some(IpAddr::V6(Ipv6Addr::from(value)))
    });
    v4.chain(v6).filter(|ip| !ip.is_unspecified()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_config_errors_match_python() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert!(ProxyAuth::from_env(env(&[]), &[]).unwrap().is_none());
        let err = ProxyAuth::from_env(env(&[("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User")]), &[])
            .unwrap_err();
        assert!(err.contains("must list the proxy's address range"));
        let wide = &[
            ("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User"),
            ("ACTIVSYNC_AUTH_PROXY_TRUSTED", "0.0.0.0/0"),
            ("ACTIVSYNC_AUTH_PROXY_USER", "me"),
        ];
        assert_eq!(
            ProxyAuth::from_env(env(wide), &[]).unwrap_err(),
            "ACTIVSYNC_AUTH_PROXY_TRUSTED must not trust every address (0.0.0.0/0)."
        );
        let docker = &[
            ("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User"),
            ("ACTIVSYNC_AUTH_PROXY_TRUSTED", "172.18.0.0/16"),
            ("ACTIVSYNC_AUTH_PROXY_USER", "me"),
        ];
        let gateway: IpAddr = "172.18.0.1".parse().unwrap();
        assert!(
            ProxyAuth::from_env(env(docker), &[gateway])
                .unwrap_err()
                .contains("default gateway (172.18.0.1)")
        );
        let proxy = ProxyAuth::from_env(env(docker), &[]).unwrap().unwrap();
        assert!(proxy.trusts(Some("172.18.4.2".parse().unwrap())));
        assert!(!proxy.trusts(Some("172.19.0.2".parse().unwrap())));
        assert_eq!(proxy.trusted[0].to_string(), "172.18.0.0/16");
    }
}
