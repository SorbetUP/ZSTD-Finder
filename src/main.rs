use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use zstd_finder::{pack_directory, Archive, EntryKind, PackOptions, DEFAULT_CHUNK_SIZE};

#[derive(Debug, Parser)]
#[command(name = "zstd-finder", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a read-only indexed Zstandard archive from a directory.
    Pack {
        source: PathBuf,
        archive: PathBuf,
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: u32,
        #[arg(short = 'l', long = "level", default_value_t = 3)]
        compression_level: i32,
        #[arg(short, long)]
        force: bool,
    },
    /// List entries without extracting them.
    List { archive: PathBuf },
    /// Verify every payload chunk and checksum.
    Verify { archive: PathBuf },
    /// Stream one archived file to stdout.
    Cat { archive: PathBuf, path: String },
    /// Read only a byte range from one archived file.
    Read {
        archive: PathBuf,
        path: String,
        offset: u64,
        length: usize,
    },
    /// Extract the whole archive or one path.
    Extract {
        archive: PathBuf,
        destination: PathBuf,
        path: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> zstd_finder::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Pack {
            source,
            archive,
            chunk_size,
            compression_level,
            force,
        } => {
            let summary = pack_directory(
                source,
                archive,
                &PackOptions {
                    chunk_size,
                    compression_level,
                    overwrite: force,
                },
            )?;
            let ratio = if summary.source_bytes == 0 {
                0.0
            } else {
                summary.archive_bytes as f64 / summary.source_bytes as f64
            };
            println!(
                "packed {} files / {} entries: {} -> {} bytes ({:.2}%), {} zstd chunks, {} stored chunks",
                summary.files,
                summary.entries,
                summary.source_bytes,
                summary.archive_bytes,
                ratio * 100.0,
                summary.compressed_chunks,
                summary.stored_chunks
            );
        }
        Command::List { archive } => {
            let archive = Archive::open(archive)?;
            for entry in archive.entries() {
                let kind = match entry.kind {
                    EntryKind::File => "file",
                    EntryKind::Directory => "dir ",
                    EntryKind::Symlink => "link",
                };
                println!("{kind} {:>12} {}", entry.size, entry.path);
            }
        }
        Command::Verify { archive } => {
            let archive = Archive::open(archive)?;
            let summary = archive.verify()?;
            println!(
                "OK: {} entries, {} files, {} chunks, {} logical bytes",
                summary.entries, summary.files, summary.chunks, summary.bytes
            );
        }
        Command::Cat { archive, path } => {
            let archive = Archive::open(archive)?;
            stream_file(&archive, &path)?;
        }
        Command::Read {
            archive,
            path,
            offset,
            length,
        } => {
            let archive = Archive::open(archive)?;
            io::stdout().write_all(&archive.read_at(&path, offset, length)?)?;
        }
        Command::Extract {
            archive,
            destination,
            path,
        } => {
            let archive = Archive::open(archive)?;
            if let Some(path) = path {
                archive.extract_path(&path, destination)?;
            } else {
                archive.extract_all(destination)?;
            }
        }
    }
    Ok(())
}

fn stream_file(archive: &Archive, path: &str) -> zstd_finder::Result<()> {
    let entry = archive.entry(path)?;
    if entry.kind != EntryKind::File {
        return Err(zstd_finder::Error::NotAFile(entry.path.clone()));
    }
    let mut offset = 0_u64;
    let mut stdout = io::stdout().lock();
    let block_size = archive.chunk_size() as usize;
    while offset < entry.size {
        let data = archive.read_at(path, offset, block_size)?;
        if data.is_empty() {
            break;
        }
        stdout.write_all(&data)?;
        offset += data.len() as u64;
    }
    Ok(())
}
