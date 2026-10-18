//! Address resolution for the handshake: port parsing, SRV records.

use std::net::SocketAddr;

use super::ProtocolError;

/// Resolves `address` to a socket address and the host/port to put in the
/// handshake. Without a port: SRV `_minecraft._tcp.<host>`, else 25565.
pub async fn resolve(address: &str) -> Result<(String, u16, Vec<SocketAddr>), ProtocolError> {
    let (host, port) = split_host_port(address);
    if let Some(port) = port {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await?
            .collect();
        return Ok((host, port, addrs));
    }
    if host.parse::<std::net::IpAddr>().is_err() {
        if let Ok(resolver) = trust_dns_resolver::TokioAsyncResolver::tokio_from_system_conf() {
            if let Ok(srv) = resolver.srv_lookup(format!("_minecraft._tcp.{host}")).await {
                if let Some(rec) = srv.iter().next() {
                    let target = rec.target().to_utf8().trim_end_matches('.').to_string();
                    let addrs: Vec<SocketAddr> =
                        tokio::net::lookup_host((target.as_str(), rec.port()))
                            .await?
                            .collect();
                    if !addrs.is_empty() {
                        return Ok((target, rec.port(), addrs));
                    }
                }
            }
        }
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 25565))
        .await?
        .collect();
    Ok((host, 25565, addrs))
}

fn split_host_port(address: &str) -> (String, Option<u16>) {
    // [v6]:port, host:port, v6 without port, host.
    if let Some(rest) = address.strip_prefix('[') {
        if let Some((h, p)) = rest.split_once("]:") {
            return (h.to_string(), p.parse().ok());
        }
        return (rest.trim_end_matches(']').to_string(), None);
    }
    match address.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().ok()),
        _ => (address.to_string(), None),
    }
}
