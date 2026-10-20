//! Restricted HTTPS tunnels for native browser sessions. Chromium retains its TLS stack.

use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::sites::Sites;

pub(crate) fn start(sites: Sites) -> Result<String, String> {
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
            std::thread::spawn(move || {
                let _ = connect(stream, &sites, bytes);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(format!("http://{address}"))
}

fn connect(mut client: TcpStream, sites: &Sites, bytes: Arc<AtomicUsize>) -> std::io::Result<()> {
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
    let addresses: Vec<_> = (host, 443).to_socket_addrs()?.collect();
    if addresses.is_empty() || addresses.iter().any(|address| !public(address.ip())) {
        client.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    let mut remote = None;
    for address in addresses.into_iter().take(4) {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
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

fn public(ip: IpAddr) -> bool {
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
