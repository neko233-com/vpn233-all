//! Upstream SOCKS5 chaining: address parsing and the client handshake.

use anyhow::{bail, Context, Result};
use std::{net::IpAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// A parsed upstream SOCKS5 proxy endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamProxy {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl UpstreamProxy {
    pub fn has_credentials(&self) -> bool {
        self.username.is_some() || self.password.is_some()
    }
}

/// Parse `host:port` or `user:pass@host:port` (the old `host:port` form keeps
/// working). The default port is 1080 when it is omitted.
pub fn parse_upstream(value: &str) -> Result<UpstreamProxy> {
    let value = value.trim();
    if value.is_empty() {
        bail!("upstream SOCKS5 address is empty");
    }
    let address = if value.contains("://") {
        value.to_owned()
    } else if value.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("socks5://[{value}]")
    } else {
        format!("socks5://{value}")
    };
    let url = url::Url::parse(&address)
        .map_err(|_| anyhow::anyhow!("invalid upstream SOCKS5 address"))?;
    if !matches!(url.scheme(), "socks5" | "socks5h")
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        bail!("invalid upstream SOCKS5 endpoint");
    }
    let host = url
        .host_str()
        .context("upstream SOCKS5 host is empty")?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = url.port().unwrap_or(1080);
    if port == 0 {
        bail!("upstream SOCKS5 port must be nonzero");
    }
    let username = if url.username().is_empty() {
        None
    } else {
        Some(decode_credential(url.username())?)
    };
    let password = url.password().map(decode_credential).transpose()?;
    Ok(UpstreamProxy {
        host,
        port,
        username,
        password,
    })
}

fn decode_credential(value: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(|byte| (byte as char).to_digit(16));
            let low = input.next().and_then(|byte| (byte as char).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                bail!("invalid percent-encoded upstream credentials");
            };
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).context("upstream SOCKS5 credentials are not UTF-8")
}

/// Parse an upstream address and apply separately configured credentials.
pub fn parse_upstream_with_credentials(
    value: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<UpstreamProxy> {
    let mut proxy = parse_upstream(value)?;
    if let Some(username) = username.and_then(non_empty) {
        proxy.username = Some(username.to_owned());
    }
    if let Some(password) = password.and_then(non_empty) {
        proxy.password = Some(password.to_owned());
    }
    Ok(proxy)
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Open a TCP stream to `target` through the upstream SOCKS5 proxy.
pub async fn connect_via_upstream(
    upstream: &UpstreamProxy,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    let connect = async {
        let mut stream = TcpStream::connect((upstream.host.as_str(), upstream.port))
            .await
            .context("cannot connect to upstream SOCKS5")?;
        negotiate(&mut stream, upstream).await?;
        request_connect(&mut stream, target_host, target_port).await?;
        Ok::<TcpStream, anyhow::Error>(stream)
    };
    timeout(UPSTREAM_TIMEOUT, connect)
        .await
        .context("upstream SOCKS5 handshake timed out")?
}

async fn negotiate(stream: &mut TcpStream, upstream: &UpstreamProxy) -> Result<()> {
    let methods: &[u8] = if upstream.has_credentials() {
        &[5, 1, 2]
    } else {
        &[5, 1, 0]
    };
    stream.write_all(methods).await?;
    let mut response = [0_u8; 2];
    stream.read_exact(&mut response).await?;
    if response[0] != 5 {
        bail!("upstream SOCKS5 replied with an unsupported version");
    }
    match response[1] {
        0x00 if !upstream.has_credentials() => Ok(()),
        0x02 => {
            let (Some(username), Some(password)) = (&upstream.username, &upstream.password) else {
                bail!("upstream SOCKS5 requires authentication but no credentials are configured");
            };
            if username.is_empty()
                || password.is_empty()
                || username.len() > u8::MAX as usize
                || password.len() > u8::MAX as usize
            {
                bail!("upstream SOCKS5 credentials are too long");
            }
            let mut request = vec![1_u8, username.len() as u8];
            request.extend_from_slice(username.as_bytes());
            request.push(password.len() as u8);
            request.extend_from_slice(password.as_bytes());
            stream.write_all(&request).await?;
            let mut auth_response = [0_u8; 2];
            stream.read_exact(&mut auth_response).await?;
            if auth_response != [1, 0] {
                bail!("upstream SOCKS5 authentication failed");
            }
            Ok(())
        }
        0xFF => bail!("upstream SOCKS5 rejected all offered authentication methods"),
        method => bail!("upstream SOCKS5 selected unsupported authentication method {method}"),
    }
}

async fn request_connect(stream: &mut TcpStream, host: &str, port: u16) -> Result<()> {
    let mut request = vec![5_u8, 1, 0];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            if host.len() > u8::MAX as usize {
                bail!("target hostname is too long for upstream SOCKS5");
            }
            request.push(3);
            request.push(host.len() as u8);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[2] != 0 {
        bail!("upstream SOCKS5 replied with an unsupported version");
    }
    if header[1] != 0 {
        bail!(
            "upstream SOCKS5 rejected the target with reply code {}",
            header[1]
        );
    }
    let address_length = match header[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            length[0] as usize
        }
        other => bail!("upstream SOCKS5 returned an invalid address type {other}"),
    };
    let mut ignored = vec![0_u8; address_length + 2];
    stream.read_exact(&mut ignored).await?;
    Ok(())
}

/// Connect directly to the target.
pub async fn connect_direct(host: &str, port: u16) -> Result<TcpStream> {
    let host = host.trim_matches(['[', ']']);
    // Tokio attempts all resolved addresses, rather than failing when only the
    // first IPv6/IPv4 result is unreachable. DNS and connect share one deadline.
    timeout(UPSTREAM_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .context("proxy target connection timed out")?
        .context("cannot connect to the proxy target")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_host_port_is_accepted() {
        let proxy = parse_upstream("127.0.0.1:7891").expect("parse");
        assert_eq!(proxy.host, "127.0.0.1");
        assert_eq!(proxy.port, 7891);
        assert!(proxy.username.is_none());
        assert!(proxy.password.is_none());
    }

    #[test]
    fn port_defaults_to_1080() {
        let proxy = parse_upstream("proxy.invalid").expect("parse");
        assert_eq!(proxy.host, "proxy.invalid");
        assert_eq!(proxy.port, 1080);
    }

    #[test]
    fn credentials_are_parsed_from_the_address() {
        let proxy = parse_upstream("alice:s3cret@example.invalid:1080").expect("parse");
        assert_eq!(proxy.username.as_deref(), Some("alice"));
        assert_eq!(proxy.password.as_deref(), Some("s3cret"));
        assert_eq!(proxy.host, "example.invalid");
        assert!(proxy.has_credentials());
    }

    #[test]
    fn explicit_credentials_override_the_address() {
        let proxy =
            parse_upstream_with_credentials("example.invalid:1080", Some("bob"), Some("pass"))
                .expect("parse");
        assert_eq!(proxy.username.as_deref(), Some("bob"));
        assert_eq!(proxy.password.as_deref(), Some("pass"));
    }

    #[test]
    fn invalid_upstream_addresses_are_rejected() {
        assert!(parse_upstream("").is_err());
        assert!(parse_upstream("   ").is_err());
        assert!(parse_upstream(":1080").is_err());
        for value in [
            "example.invalid:0",
            "example.invalid:not-a-port",
            "http://example.invalid:1080",
            "socks5://example.invalid:1080/path",
        ] {
            assert!(parse_upstream(value).is_err());
        }
    }

    #[test]
    fn upstream_uri_percent_encoding_and_ipv6_are_supported() {
        let proxy = parse_upstream("socks5://user:change%3Ame%40value@[2001:db8::1]:1080").unwrap();
        assert_eq!(proxy.host, "2001:db8::1");
        assert_eq!(proxy.password.as_deref(), Some("change:me@value"));
        assert_eq!(parse_upstream("2001:db8::1").unwrap().port, 1080);
    }
}
