//! Restricted HTTPS tunnels for native browser sessions. Chromium retains its TLS stack.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::resolve::lookup;
use crate::sites::Sites;

pub(crate) fn start(sites: Sites, proxy: Option<crate::OutboundProxy>) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let sites = Arc::new(sites);
    let active = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if active.fetch_add(1, Ordering::SeqCst) >= 32 {
                active.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let sites = Arc::clone(&sites);
            let active = Arc::clone(&active);
            let bytes = Arc::clone(&bytes);
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                let _ = connect(stream, &sites, bytes, proxy.as_ref());
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(format!("http://{address}"))
}

fn connect(
    mut client: TcpStream,
    sites: &Sites,
    bytes: Arc<AtomicUsize>,
    proxy: Option<&crate::OutboundProxy>,
) -> std::io::Result<()> {
    configure(&client)?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
        let mut byte = [0];
        client.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let request = String::from_utf8_lossy(&header);
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next();
    let authority = parts.next().unwrap_or("");
    let host = authority.strip_suffix(":443").unwrap_or("");
    if !header.ends_with(b"\r\n\r\n") || method != Some("CONNECT") || !sites.allows(host) {
        client.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    // Resolve once and connect to the checked address, preventing rebinding
    // between validation and connection. Never reach local services.
    let Ok(addresses) = lookup(host) else {
        client.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    };
    let mut remote = None;
    for mut address in addresses.into_iter().take(4) {
        address.set_port(443);
        let connection = match proxy {
            Some(proxy) => via_proxy(proxy, address),
            None => TcpStream::connect_timeout(&address, Duration::from_secs(3)),
        };
        if let Ok(stream) = connection {
            remote = Some(stream);
            break;
        }
    }
    let remote = remote.ok_or_else(|| std::io::Error::other("couldn't connect to the site"))?;
    configure(&remote)?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    let upstream = client.try_clone()?;
    let downstream = remote.try_clone()?;
    let upload_bytes = Arc::clone(&bytes);
    let upload = std::thread::spawn(move || copy(upstream, remote, &upload_bytes));
    copy(downstream, client, &bytes);
    let _ = upload.join();
    Ok(())
}

fn via_proxy(
    proxy: &crate::OutboundProxy,
    destination: std::net::SocketAddr,
) -> std::io::Result<TcpStream> {
    let mut remote = None;
    for address in proxy.address().to_socket_addrs()?.take(4) {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
            remote = Some(stream);
            break;
        }
    }
    let mut stream = remote.ok_or_else(|| std::io::Error::other("outbound proxy unavailable"))?;
    configure(&stream)?;
    // Send the checked IP, not a hostname the proxy could resolve differently.
    // Chromium still sends the original hostname in its TLS handshake.
    write!(
        stream,
        "CONNECT {destination} HTTP/1.1\r\nHost: {destination}\r\n"
    )?;
    if let Some(auth) = proxy.authorization() {
        write!(stream, "Proxy-Authorization: Basic {auth}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let response = String::from_utf8_lossy(&header);
    if !header.ends_with(b"\r\n\r\n")
        || response
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            != Some("200")
    {
        return Err(std::io::Error::other("outbound proxy rejected the tunnel"));
    }
    Ok(stream)
}

fn configure(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))
}

fn copy(mut from: TcpStream, mut to: TcpStream, total: &AtomicUsize) {
    let mut buffer = [0; 16 << 10];
    while let Ok(length) = from.read(&mut buffer) {
        if length == 0 || total.fetch_add(length, Ordering::SeqCst) + length > 128 << 20 {
            break;
        }
        if to.write_all(&buffer[..length]).is_err() {
            break;
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}
