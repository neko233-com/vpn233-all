//! Export synthetic configurations for a separately verified `mihomo -t`.
//! No network requests, real nodes or deployment credentials are used.

#[path = "../tests/support/mihomo_fixtures.rs"]
mod fixtures;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(std::path::PathBuf::from)
        .ok_or("usage: mihomo_fixtures <ignored-output-directory>")?;
    if args.next().is_some() {
        return Err("usage: mihomo_fixtures <ignored-output-directory>".into());
    }
    std::fs::create_dir_all(&root)?;
    let cases = fixtures::configurations();
    for (name, body) in &cases {
        // A fresh directory avoids accidentally reusing stale fixtures.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("{name}.yaml")))?;
        std::io::Write::write_all(&mut file, body.as_bytes())?;
    }
    println!(
        "Exported {} synthetic Mihomo parser configurations",
        cases.len()
    );
    Ok(())
}
