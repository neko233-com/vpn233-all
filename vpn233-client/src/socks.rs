//! RFC 1928 SOCKS5 server with optional RFC 1929 username/password auth.

use crate::{
    auth,
    config::Config,
    state::{relay, AppState},
    upstream,
};
use anyhow::{bail, Context, Result};
use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Serve one accepted SOCKS5 connection.
pub async fn handle_socks(
    stream: TcpStream,
    _peer: SocketAddr,
    state: Arc<AppState>,
) -> Result<()> {
    state.begin_connection().await;
    let result = handle_socks_inner(stream, &state).await;
    state.end_connection().await;
    if let Err(error) = &result {
        tracing::warn!(error_kind = error_kind(error), "SOCKS5 connection failed");
    }
    result
}

async fn handle_socks_inner(mut stream: TcpStream, state: &AppState) -> Result<()> {
    let config = state.config_snapshot().await;
    let (host, port) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        handshake(&mut stream, &config),
    )
    .await
    .context("SOCKS5 handshake timed out")??;
    let mut remote = match connect_target(&config, &host, port).await {
        Ok(remote) => remote,
        Err(error) => {
            let _ = write_reply(&mut stream, 0x05).await;
            return Err(error);
        }
    };
    write_reply(&mut stream, 0x00).await?;
    relay(&mut stream, &mut remote, state, &host).await
}

async fn handshake(stream: &mut TcpStream, config: &Config) -> Result<(String, u16)> {
    let mut greeting = [0_u8; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting[0] != 5 {
        bail!("unsupported SOCKS version");
    }
    let mut methods = vec![0_u8; greeting[1] as usize];
    stream.read_exact(&mut methods).await?;
    if config.socks_auth_enabled() {
        if !methods.contains(&0x02) {
            stream.write_all(&[5, 0xFF]).await?;
            bail!("client does not offer username/password authentication");
        }
        stream.write_all(&[5, 2]).await?;
        authenticate(stream, config).await?;
    } else {
        if !methods.contains(&0x00) {
            stream.write_all(&[5, 0xFF]).await?;
            bail!("client does not offer the no-auth method");
        }
        stream.write_all(&[5, 0]).await?;
    }

    let mut request = [0_u8; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != 5 || request[2] != 0 {
        bail!("unsupported SOCKS version in request");
    }
    let host = read_address(stream, request[3]).await?;
    let mut port_bytes = [0_u8; 2];
    stream.read_exact(&mut port_bytes).await?;
    let port = u16::from_be_bytes(port_bytes);
    match request[1] {
        1 => {}
        2 | 3 => {
            write_reply(stream, 0x07).await?;
            bail!("only SOCKS5 CONNECT is supported");
        }
        command => {
            write_reply(stream, 0x07).await?;
            bail!("unsupported SOCKS5 command {command}");
        }
    }

    if host.is_empty() || port == 0 {
        write_reply(stream, 0x08).await?;
        bail!("SOCKS5 target endpoint is invalid");
    }
    Ok((host, port))
}

async fn authenticate(stream: &mut TcpStream, config: &Config) -> Result<()> {
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await?;
    if header[0] != 1 {
        let _ = stream.write_all(&[1, 1]).await;
        bail!("unsupported SOCKS5 authentication version");
    }
    let mut username = vec![0_u8; header[1] as usize];
    stream.read_exact(&mut username).await?;
    let mut password_length = [0_u8; 1];
    stream.read_exact(&mut password_length).await?;
    let mut password = vec![0_u8; password_length[0] as usize];
    stream.read_exact(&mut password).await?;
    let username = String::from_utf8(username).context("SOCKS5 username is not UTF-8")?;
    let password = String::from_utf8(password).context("SOCKS5 password is not UTF-8")?;
    if auth::credentials_match(
        &username,
        &password,
        &config.socks_user,
        &config.socks_password,
    ) {
        stream.write_all(&[1, 0]).await?;
        Ok(())
    } else {
        stream.write_all(&[1, 1]).await?;
        bail!("SOCKS5 authentication failed");
    }
}

async fn read_address(stream: &mut TcpStream, address_type: u8) -> Result<String> {
    match address_type {
        1 => {
            let mut bytes = [0_u8; 4];
            stream.read_exact(&mut bytes).await?;
            Ok(Ipv4Addr::from(bytes).to_string())
        }
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            let mut name = vec![0_u8; length[0] as usize];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name).context("SOCKS5 target host is not UTF-8")
        }
        4 => {
            let mut bytes = [0_u8; 16];
            stream.read_exact(&mut bytes).await?;
            Ok(Ipv6Addr::from(bytes).to_string())
        }
        other => bail!("unsupported SOCKS5 address type {other}"),
    }
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

async fn write_reply(stream: &mut TcpStream, code: u8) -> Result<()> {
    stream.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
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
    if error.to_string().contains("authentication") {
        return "auth";
    }
    "protocol"
}
