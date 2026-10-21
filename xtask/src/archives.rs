//! Deterministic public bundles; archive creation never invokes an interpreter.
use anyhow::{anyhow, bail, Context, Result};
use flate2::{Compression, GzBuilder};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipWriter};

const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
struct Member {
    name: String,
    directory: bool,
    mode: u32,
    data: Vec<u8>,
}

fn collect(root: &Path, path: &Path, members: &mut Vec<Member>, total: &mut u64) -> Result<()> {
    if members.len() >= MAX_ENTRIES {
        bail!("archive input exceeds member bound");
    }
    let metadata = fs::symlink_metadata(path).context("archive input metadata unavailable")?;
    if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
        bail!("archive input must contain only regular files and directories");
    }
    let name = path
        .strip_prefix(
            root.parent()
                .ok_or_else(|| anyhow!("invalid bundle root"))?,
        )?
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 archive input path"))?
        .replace('\\', "/");
    let mode = if metadata.is_dir()
        || matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("sh" | "exe")
        )
        || path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "bin")
    {
        0o755
    } else {
        0o644
    };
    let mut data = Vec::new();
    if metadata.is_file() {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000);
        }
        let input = options
            .open(path)
            .context("archive source could not be opened")?;
        let opened = input.metadata()?;
        if !opened.is_file()
            || opened.file_type().is_symlink()
            || opened.len() > crate::privacy::MAX_INPUT_BYTES
        {
            bail!("invalid or oversized archive source");
        }
        input
            .take(crate::privacy::MAX_INPUT_BYTES + 1)
            .read_to_end(&mut data)?;
        *total = total
            .checked_add(data.len() as u64)
            .ok_or_else(|| anyhow!("archive input size overflow"))?;
        if data.len() as u64 > crate::privacy::MAX_INPUT_BYTES || *total > MAX_TOTAL {
            bail!("archive input exceeds size bound");
        }
    }
    members.push(Member {
        name,
        directory: metadata.is_dir(),
        mode,
        data,
    });
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(root, &entry?.path(), members, total)?;
        }
    }
    Ok(())
}

fn intended_output(path: &Path) -> Result<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("archive output must not contain parent traversal");
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut cursor = absolute.as_path();
    let mut missing = Vec::new();
    while !cursor.exists() {
        missing.push(
            cursor
                .file_name()
                .ok_or_else(|| anyhow!("invalid archive output path"))?
                .to_os_string(),
        );
        cursor = cursor
            .parent()
            .ok_or_else(|| anyhow!("invalid archive output path"))?;
    }
    let mut resolved = crate::support::canonical_path(cursor)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub fn create(bundle: &Path, output: &Path, epoch: u64) -> Result<()> {
    if fs::symlink_metadata(bundle)?.file_type().is_symlink() {
        bail!("archive bundle root must not be a symlink");
    }
    let bundle = crate::support::canonical_path(bundle)?;
    if !bundle.is_dir() {
        bail!("archive bundle must be a directory");
    }
    let output = intended_output(output)?;
    if output.starts_with(&bundle) {
        bail!("archive output must be outside the input bundle");
    }
    let epoch = u32::try_from(epoch)
        .map_err(|_| anyhow!("SOURCE_DATE_EPOCH exceeds archive timestamp range"))?;
    let stamp = time::OffsetDateTime::from_unix_timestamp(i64::from(epoch.max(315_532_800)))?;
    let date = DateTime::from_date_and_time(
        stamp.year() as u16,
        stamp.month() as u8,
        stamp.day(),
        stamp.hour(),
        stamp.minute(),
        stamp.second(),
    )
    .map_err(|_| anyhow!("invalid ZIP timestamp"))?;
    let mut members = Vec::new();
    collect(&bundle, &bundle, &mut members, &mut 0)?;
    members.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    // Snapshot once: a concurrent source edit cannot produce differing payloads
    // in the tar and ZIP representations of this bundle.
    fs::create_dir_all(&output)?;
    let mut tar_file = tempfile::NamedTempFile::new_in(&output)?;
    let mut zip_file = tempfile::NamedTempFile::new_in(&output)?;
    {
        let encoder = GzBuilder::new()
            .mtime(epoch)
            .operating_system(255)
            .write(tar_file.as_file_mut(), Compression::new(1));
        let mut archive = tar::Builder::new(encoder);
        for member in &members {
            let mut header = tar::Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            header.set_username("")?;
            header.set_groupname("")?;
            header.set_mtime(u64::from(epoch));
            header.set_mode(member.mode);
            header.set_entry_type(if member.directory {
                tar::EntryType::Directory
            } else {
                tar::EntryType::Regular
            });
            header.set_size(member.data.len() as u64);
            header.set_cksum();
            archive.append_data(&mut header, &member.name, Cursor::new(&member.data))?;
        }
        archive.into_inner()?.finish()?;
    }
    {
        let mut archive = ZipWriter::new(zip_file.as_file_mut());
        for member in &members {
            let options = SimpleFileOptions::default()
                // zip 9 chooses the host system by default; the DOS setting on
                // Windows discards executable bits even with unix_permissions.
                .system(zip::System::Unix)
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(1))
                .last_modified_time(date)
                .unix_permissions(member.mode);
            if member.directory {
                archive.add_directory(format!("{}/", member.name), options)?;
            } else {
                archive.start_file(&member.name, options)?;
                archive.write_all(&member.data)?;
            }
        }
        archive.finish()?;
    }
    tar_file.as_file().sync_all()?;
    zip_file.as_file().sync_all()?;
    let name = bundle
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid archive bundle name"))?;
    tar_file
        .persist(output.join(format!("{name}.tar.gz")))
        .map_err(|_| anyhow!("tar archive publication failed"))?;
    zip_file
        .persist(output.join(format!("{name}.zip")))
        .map_err(|_| anyhow!("ZIP archive publication failed"))?;
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    if args.len() != 2 {
        bail!("usage: archive BUNDLE OUTPUT");
    }
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .unwrap_or_else(|_| "315532800".into())
        .parse::<u64>()
        .map_err(|_| anyhow!("invalid SOURCE_DATE_EPOCH"))?;
    create(Path::new(&args[0]), Path::new(&args[1]), epoch)
        .map_err(|_| anyhow!("archive creation failed; input contents redacted"))
}
