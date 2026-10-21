//! HTTP proxy: CONNECT tunnels, absolute-form forwarding, Basic auth and PAC.

use crate::{
    auth,
    config::Config,
    state::{relay, split_host_port, AppState},
    upstream,
};
use anyhow::{bail, Context, Result};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Serve one accepted HTTP proxy connection.
pub async fn handle_http(stream: TcpStream, _peer: SocketAddr, state: Arc<AppState>) -> Result<()> {
    state.begin_connection().await;
    let result = handle_http_inner(stream, &state).await;
    state.end_connection().await;
    if let Err(error) = &result {
        tracing::warn!(
            error_kind = error_kind(error),
            "HTTP proxy connection failed"
        );
    }
    result
}

async fn handle_http_inner(mut stream: TcpStream, state: &AppState) -> Result<()> {
    let config = state.config_snapshot().await;
    let (head, tail) = timeout(HEADER_READ_TIMEOUT, read_request_head(&mut stream))
        .await
        .context("timed out while reading the HTTP proxy request")??;
    let first_line = head.lines().next().context("missing HTTP request line")?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().context("missing HTTP method")?.to_owned();
    let target = parts.next().context("missing HTTP target")?.to_owned();
    let headers = parse_headers(&head);

    if config.http_auth_enabled() {
        let provided = header_value(&headers, "proxy-authorization");
        if !auth::proxy_authorization_matches(provided, &config.http_user, &config.http_password) {
            write_response(
                &mut stream,
                407,
                "Proxy Authentication Required",
                &[("Proxy-Authenticate", "Basic realm=\"vpn233\"")],
                "proxy authentication required\n",
            )
            .await?;
            return Ok(());
        }
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(&target, 443);
        let mut remote = match connect_target(&config, &host, port).await {
            Ok(remote) => remote,
            Err(error) => {
                write_response(
                    &mut stream,
                    502,
                    "Bad Gateway",
                    &[],
                    "cannot connect to the proxy target\n",
                )
                .await?;
                return Err(error);
            }
        };
        stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        remote.write_all(&tail).await?;
        return relay(&mut stream, &mut remote, state, &host).await;
    }

    let url = match url::Url::parse(&target) {
        Ok(url) => url,
        Err(_) => {
            write_response(
                &mut stream,
                400,
                "Bad Request",
                &[],
                "the HTTP proxy requires an absolute-form request target\n",
            )
            .await?;
            return Ok(());
        }
    };
    if !url.has_host() {
        write_response(
            &mut stream,
            400,
            "Bad Request",
            &[],
            "the HTTP proxy requires an absolute-form request target\n",
        )
        .await?;
        return Ok(());
    }
    if url.scheme() != "http" {
        write_response(
            &mut stream,
            502,
            "Bad Gateway",
            &[],
            "HTTPS targets must use CONNECT\n",
        )
        .await?;
        bail!("unsupported absolute-form scheme");
    }
    let host = url.host_str().unwrap_or_default().to_owned();
    let port = url.port_or_known_default().unwrap_or(80);
    let mut remote = match connect_target(&config, &host, port).await {
        Ok(remote) => remote,
        Err(error) => {
            write_response(
                &mut stream,
                502,
                "Bad Gateway",
                &[],
                "cannot connect to the proxy target\n",
            )
            .await?;
            return Err(error);
        }
    };
    let origin_request = build_origin_request(&method, &url, &headers, &tail);
    remote.write_all(&origin_request).await?;
    relay(&mut stream, &mut remote, state, &host).await
}

async fn connect_target(config: &Config, host: &str, port: u16) -> Result<TcpStream> {
    if config.use_upstream() {
        let upstream_proxy = upstream::parse_upstream_with_credentials(
            &config.upstream_socks5,
            Some(&config.upstream_socks5_user),
            Some(&config.upstream_socks5_password),
        )?;
        upstream::connect_via_upstream(&upstream_proxy, host, port).await
    } else {
        upstream::connect_direct(host, port).await
    }
}

async fn read_request_head(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut buffer = Vec::with_capacity(8 * 1024);
    let mut chunk = [0_u8; 4 * 1024];
    loop {
        if buffer.len() > MAX_HEADER_BYTES {
            bail!("HTTP request headers exceed the limit");
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("client closed the connection before sending HTTP headers");
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
            if position > MAX_HEADER_BYTES {
                bail!("HTTP request headers exceed the limit");
            }
            let head = String::from_utf8_lossy(&buffer[..position]).to_string();
            let tail = buffer[position + 4..].to_vec();
            return Ok((head, tail));
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_headers(head: &str) -> Vec<(String, String)> {
    head.lines()
        .skip(1)
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn build_origin_request(
    method: &str,
    url: &url::Url,
    headers: &[(String, String)],
    tail: &[u8],
) -> Vec<u8> {
    let mut path = if url.path().is_empty() {
        "/".to_owned()
    } else {
        url.path().to_owned()
    };
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let host_header = match header_value(headers, "host") {
        Some(value) => value.to_owned(),
        None => {
            let host = url.host_str().unwrap_or_default();
            match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            }
        }
    };
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host_header}\r\n");
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host" | "proxy-authorization" | "proxy-connection" | "connection" | "content-length"
        ) {
            continue;
        }
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    if let Some(content_length) = header_value(headers, "content-length") {
        request.push_str("Content-Length: ");
        request.push_str(content_length);
        request.push_str("\r\n");
    }
    request.push_str("Connection: close\r\n\r\n");
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(tail);
    bytes
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

/// Classify an error for privacy-preserving logs (never includes the target).
fn error_kind(error: &anyhow::Error) -> &'static str {
    if error.downcast_ref::<std::io::Error>().is_some() {
        return "io";
    }
    if error.to_string().contains("upstream") {
        return "upstream";
    }
    if error.to_string().contains("scheme") {
        return "scheme";
    }
    "protocol"
}
