use anyhow::{anyhow, bail, ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand_core::{OsRng, RngCore};
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener},
};

fn input() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "fixture input exceeds limit"
    );
    Ok(bytes)
}
pub(super) fn run(args: &[String]) -> Result<()> {
    let command = args.first().map(String::as_str).unwrap_or("");
    let arg = |index| {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("missing E2E helper argument"))
    };
    match command {
        "random-hex" => {
            let count: usize = arg(1)?.parse()?;
            ensure!(
                (1..=1024).contains(&count),
                "random byte count outside fixture bounds"
            );
            let mut bytes = vec![0; count];
            OsRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| anyhow!("OS random source failed"))?;
            println!(
                "{}",
                bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
        }
        "free-ports" => {
            let count: usize = arg(1)?.parse()?;
            ensure!((1..=32).contains(&count), "invalid fixture port count");
            let listeners = (0..count)
                .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
                .collect::<std::io::Result<Vec<_>>>()?;
            println!(
                "{}",
                listeners
                    .iter()
                    .map(|l| l.local_addr().map(|a| a.port().to_string()))
                    .collect::<std::io::Result<Vec<_>>>()?
                    .join(" ")
            );
        }
        "base64-encode" => println!("{}", STANDARD.encode(input()?)),
        "base64-decode" => {
            let encoded: Vec<_> = input()?
                .into_iter()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            let decoded = STANDARD
                .decode(encoded)
                .map_err(|_| anyhow!("invalid fixture Base64"))?;
            io::stdout().write_all(&decoded)?;
        }
        "validate-yaml" => {
            serde_yaml::from_slice::<serde_yaml::Value>(&input()?)
                .map_err(|_| anyhow!("invalid fixture YAML"))?;
        }
        "validate-links" => {
            let expected: usize = arg(1)?.parse()?;
            let bytes = input()?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| anyhow!("invalid fixture links encoding"))?;
            let schemes = [
                "vless",
                "vmess",
                "trojan",
                "ss",
                "ssr",
                "hysteria2",
                "hy2",
                "tuic",
                "anytls",
                "socks5",
                "socks",
                "http",
                "https",
            ];
            ensure!(
                text.lines().count() == expected
                    && text.lines().all(|l| l
                        .split_once("://")
                        .is_some_and(|(s, _)| schemes.contains(&s))),
                "fixture link count or scheme mismatch"
            );
        }
        "validate-json" | "json-get" | "json-length" | "json-contains" | "provider-online" => {
            let value: Value =
                serde_json::from_slice(&input()?).map_err(|_| anyhow!("invalid fixture JSON"))?;
            if command == "validate-json" {
                return Ok(());
            }
            if command == "provider-online" {
                let id = arg(1)?;
                println!(
                    "{}",
                    value["providers"]
                        .as_array()
                        .is_some_and(|items| items
                            .iter()
                            .any(|p| p["provider_id"].as_str() == Some(id)
                                && p["online"].as_bool() == Some(true)
                                && p["node_count"].as_u64().unwrap_or(0) > 0))
                );
                return Ok(());
            }
            let selected = value
                .pointer(arg(1)?)
                .ok_or_else(|| anyhow!("fixture JSON field missing"))?;
            match command {
                "json-get" => match selected {
                    Value::String(s) => println!("{s}"),
                    Value::Number(n) => println!("{n}"),
                    Value::Bool(b) => println!("{b}"),
                    _ => bail!("fixture JSON field is not scalar"),
                },
                "json-length" => println!(
                    "{}",
                    selected
                        .as_array()
                        .map(Vec::len)
                        .or_else(|| selected.as_object().map(serde_json::Map::len))
                        .ok_or_else(|| anyhow!("fixture JSON field has no length"))?
                ),
                "json-contains" => println!(
                    "{}",
                    selected
                        .as_array()
                        .ok_or_else(|| anyhow!("fixture JSON field is not an array"))?
                        .iter()
                        .any(|v| v.as_str() == Some(arg(2).unwrap_or("")))
                ),
                _ => unreachable!(),
            }
        }
        _ => bail!("unknown E2E helper command"),
    }
    Ok(())
}
