//! Name resolution for extension requests. A declared site is a name, and
//! whoever controls the name can point it anywhere, including at the
//! server's own network, so the host resolves names itself and refuses
//! addresses that aren't public.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// The resolver for every client that reaches an extension's sites. The
/// addresses it returns are the ones reqwest connects to, so a record can't
/// change between the check and the connection.
pub(crate) struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            // The system resolver blocks, as reqwest's own resolver does.
            let addresses = tokio::task::spawn_blocking(move || lookup(&host))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result)
                .map_err(Box::<dyn std::error::Error + Send + Sync>::from)?;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// Resolves `host` with the system resolver, with port 0, and refuses it
/// when any of its addresses isn't public. One private record among public
/// ones is refused too, since a client could pick any of them.
pub(crate) fn lookup(host: &str) -> Result<Vec<SocketAddr>, String> {
    let addresses: Vec<SocketAddr> = (host, 0)
        .to_socket_addrs()
        .map_err(|error| format!("`{host}` couldn't be resolved: {error}"))?
        .collect();
    if addresses.is_empty() {
        return Err(format!("`{host}` has no address"));
    }
    if let Some(address) = addresses.iter().find(|address| !public(address.ip())) {
        return Err(format!(
            "`{host}` resolves to {}, which isn't a public address",
            address.ip()
        ));
    }
    Ok(addresses)
}

/// Whether an address is routable on the internet rather than loopback,
/// private, link-local, reserved or otherwise local to the server.
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && !ip.is_broadcast()
                && !ip.is_unspecified()
                && a != 0
                && a < 224
                && !(a == 100 && (64..128).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 0)
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] < 0x200 || segments[1] == 0xdb8))
        }
    }
}

#[cfg(test)]
mod tests {
    /// `localhost` resolves without the network, so this holds anywhere.
    #[test]
    fn refuses_a_name_on_the_servers_network() {
        let error = super::lookup("localhost").unwrap_err();
        assert!(error.contains("isn't a public address"), "{error}");
    }
}
