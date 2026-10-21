use super::{option, private_write};
use anyhow::{anyhow, ensure, Result};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::{timeout, Duration},
};

struct ReadyFile(Option<PathBuf>);
impl ReadyFile {
    fn remove(&mut self) -> Result<()> {
        if let Some(path) = self.0.take() {
            std::fs::remove_file(path).map_err(|e| {
                anyhow!(
                    "echo fixture cleanup error errno={}",
                    e.raw_os_error().unwrap_or(0)
                )
            })?;
        }
        Ok(())
    }
}
impl Drop for ReadyFile {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let port: u16 = option(args, "--port")?
        .unwrap_or_else(|| "18080".into())
        .parse()
        .map_err(|_| anyhow!("echo fixture error: invalid port errno=0"))?;
    let bind: IpAddr = option(args, "--bind")?
        .unwrap_or_else(|| "127.0.0.1".into())
        .parse()
        .map_err(|_| anyhow!("echo fixture error: invalid bind errno=0"))?;
    ensure!(
        bind.is_loopback(),
        "echo fixture must bind a numeric loopback address"
    );
    let marker = option(args, "--marker")?.unwrap_or_else(|| "vpn233-e2e-echo".into());
    let ready = option(args, "--ready-file")?.map(PathBuf::from);
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = TcpListener::bind(SocketAddr::new(bind, port)).await.map_err(|e| anyhow!("echo fixture bind error errno={}", e.raw_os_error().unwrap_or(0)))?;
        if let Some(path) = &ready { private_write(path, format!("{}\n", listener.local_addr()?.port()).as_bytes()).map_err(|_| anyhow!("echo fixture readiness error errno=0"))?; }
        let mut owned = ReadyFile(ready);
        let mut workers = JoinSet::new();
        println!("echo fixture ready");
        #[cfg(unix)] let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        #[cfg(unix)] let stop = async { tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} } };
        #[cfg(not(unix))] let stop = async { let _ = tokio::signal::ctrl_c().await; };
        tokio::pin!(stop);
        loop {
            tokio::select! {
                _ = &mut stop => break,
                connection = listener.accept() => {
                    let (stream, _) = connection.map_err(|e| anyhow!("echo fixture accept error errno={}", e.raw_os_error().unwrap_or(0)))?;
                    let marker = marker.clone(); workers.spawn(async move { let _ = serve(stream, &marker).await; });
                },
                _ = workers.join_next(), if !workers.is_empty() => {},
            }
        }
        workers.abort_all(); while workers.join_next().await.is_some() {}
        owned.remove()
    })
}

async fn serve(stream: TcpStream, marker: &str) -> Result<()> {
    let mut stream = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let count = timeout(Duration::from_secs(5), stream.read_line(&mut line)).await??;
        if count == 0 {
            break;
        }
        ensure!(line.len() <= 8192, "echo request limit");
        let words: Vec<_> = line.split_whitespace().collect();
        ensure!(words.len() == 3, "invalid echo request");
        let method = words[0].to_owned();
        let path = words[1].to_owned();
        let mut host = String::new();
        let mut total = count;
        let mut close = false;
        loop {
            line.clear();
            total += timeout(Duration::from_secs(5), stream.read_line(&mut line)).await??;
            ensure!(total < 65536, "echo header limit");
            if line == "\r\n" || line == "\n" {
                break;
            }
            ensure!(!line.is_empty(), "incomplete echo headers");
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("host") {
                    host = value.trim().into();
                }
                if name.eq_ignore_ascii_case("connection")
                    && value.trim().eq_ignore_ascii_case("close")
                {
                    close = true;
                }
            }
        }
        let body = if method == "HEAD" {
            Vec::new()
        } else {
            serde_json::to_vec(
                &serde_json::json!({"marker":marker,"method":method,"path":path,"host":host}),
            )?
        };
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n", body.len());
        stream.get_mut().write_all(response.as_bytes()).await?;
        stream.get_mut().write_all(&body).await?;
        if close {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_is_numeric() {
        assert_eq!(
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST).to_string(),
            "127.0.0.1"
        );
    }
    #[test]
    fn cleanup_failure_reports_no_path_and_preserves_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private-ready");
        std::fs::create_dir(&path).unwrap();
        let mut file = ReadyFile(Some(path.clone()));
        let error = file.remove().unwrap_err().to_string();
        assert!(error.starts_with("echo fixture cleanup error errno="));
        assert!(!error.contains(&path.to_string_lossy().into_owned()));
        assert!(path.is_dir());
    }
}
