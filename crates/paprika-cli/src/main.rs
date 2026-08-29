use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use paprika_core::{Mode, OptimizationOptions};
use paprika_epub::{EpubOptions, convert_pdf_to_epub_owned};
use paprika_pdf::optimize_pdf_owned;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Parser)]
#[command(
    name = "paprika",
    version,
    about = "Make PDFs readable on small screens",
    long_about = "Paprika converts born-digital PDFs to compact, reflowable EPUB 3 documents with selectable text. An experimental raster PDF mode remains available for scans and difficult layouts. Documents never leave this machine."
)]
struct Arguments {
    /// Source PDF.
    input: PathBuf,

    /// Destination file. Defaults to <input>.paprika.epub.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output format. Defaults to EPUB unless --output ends in .pdf.
    #[arg(long, value_enum)]
    format: Option<OutputFormat>,

    /// EPUB title. Defaults to the input file name.
    #[arg(long)]
    title: Option<String>,

    /// EPUB language as a BCP 47 tag.
    #[arg(long, default_value = "en")]
    language: String,

    /// Omit embedded raster images from EPUB output.
    #[arg(long)]
    no_images: bool,

    /// Layout strategy for experimental raster PDF output.
    #[arg(long, value_enum, default_value_t = CliMode::FitWidth)]
    mode: CliMode,

    /// Raster PDF output width in pixels.
    #[arg(long, default_value_t = 758)]
    width: u32,

    /// Raster PDF output height in pixels.
    #[arg(long, default_value_t = 1024)]
    height: u32,

    /// Raster PDF output resolution in dots per inch.
    #[arg(long, default_value_t = 167)]
    dpi: u32,

    /// Resolution used to render raster PDF source pages.
    #[arg(long, default_value_t = 144)]
    source_dpi: u32,

    /// Raster PDF output margin in pixels.
    #[arg(long, default_value_t = 24)]
    margin: u32,

    /// Approximate graphical text size in raster reflow mode.
    #[arg(long, default_value_t = 12.0)]
    font_size: f32,

    /// Grayscale value below which pixels count as content in raster PDF mode.
    #[arg(long, default_value_t = 245)]
    threshold: u8,

    /// Maximum detected columns for raster PDF output (1 or 2).
    #[arg(long, default_value_t = 2)]
    columns: u8,

    /// Replace an existing destination file.
    #[arg(long)]
    force: bool,

    /// Suppress the conversion summary.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    #[default]
    Epub,
    Pdf,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CliMode {
    Reflow,
    #[default]
    FitWidth,
    FitPage,
}

impl From<CliMode> for Mode {
    fn from(value: CliMode) -> Self {
        match value {
            CliMode::Reflow => Mode::Reflow,
            CliMode::FitWidth => Mode::FitWidth,
            CliMode::FitPage => Mode::FitPage,
        }
    }
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let format = resolve_format(arguments.format, arguments.output.as_deref());
    let output = arguments
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(&arguments.input, format));

    if arguments.input == output {
        bail!("input and output paths must differ");
    }
    if output.exists() && !arguments.force {
        bail!(
            "{} already exists; choose another output or pass --force",
            output.display()
        );
    }

    let started = Instant::now();
    let input = std::fs::read(&arguments.input)
        .with_context(|| format!("could not read {}", arguments.input.display()))?;

    let summary = match format {
        OutputFormat::Epub => {
            let title = arguments
                .title
                .unwrap_or_else(|| input_title(&arguments.input));
            let result = convert_pdf_to_epub_owned(
                input,
                EpubOptions {
                    title,
                    language: arguments.language,
                    include_images: !arguments.no_images,
                    ..Default::default()
                },
            )
            .context("EPUB conversion failed")?;
            write_destination(&output, &result.bytes, arguments.force)?;
            for warning in &result.warnings {
                eprintln!("warning: {warning}");
            }
            format!(
                "{} source page(s) → EPUB with {} text page(s) and {} image(s), {} bytes",
                result.source_pages,
                result.text_pages,
                result.image_count,
                result.bytes.len()
            )
        }
        OutputFormat::Pdf => {
            let options = OptimizationOptions {
                mode: arguments.mode.into(),
                width: arguments.width,
                height: arguments.height,
                dpi: arguments.dpi,
                source_dpi: arguments.source_dpi,
                margin: arguments.margin,
                font_size: arguments.font_size,
                threshold: arguments.threshold,
                columns: arguments.columns,
            };
            options.validate()?;
            let result =
                optimize_pdf_owned(input, options).context("raster PDF conversion failed")?;
            write_destination(&output, &result.bytes, arguments.force)?;
            format!(
                "{} source page(s) → {} raster PDF page(s), {} bytes",
                result.source_pages,
                result.output_pages,
                result.bytes.len()
            )
        }
    };

    if !arguments.quiet {
        eprintln!(
            "{} in {:.2}s\n{}",
            summary,
            started.elapsed().as_secs_f32(),
            output.display()
        );
    }
    Ok(())
}

fn write_destination(destination: &Path, bytes: &[u8], force: bool) -> Result<()> {
    let result = write_destination_with(destination, force, |file| file.write_all(bytes));
    match result {
        Ok(()) => Ok(()),
        Err(error) if !force && error.kind() == io::ErrorKind::AlreadyExists => bail!(
            "{} already exists; choose another output or pass --force",
            destination.display()
        ),
        Err(error) => {
            Err(error).with_context(|| format!("could not write {}", destination.display()))
        }
    }
}

fn write_destination_with(
    destination: &Path,
    force: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let (staged, mut file) = StagedFile::create(destination)?;
    write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    staged.commit(destination, force)?;
    sync_parent_directory(destination)
}

struct StagedFile {
    path: PathBuf,
    committed: bool,
}

impl StagedFile {
    fn create(destination: &Path) -> io::Result<(Self, File)> {
        let file_name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination must have a file name",
            )
        })?;
        let parent = destination_parent(destination);

        for _ in 0..100 {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = OsString::from(".");
            temporary_name.push(file_name);
            temporary_name.push(format!(".paprika-{}-{sequence}.tmp", std::process::id()));
            let path = parent.join(temporary_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            path,
                            committed: false,
                        },
                        file,
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a temporary output file",
        ))
    }

    fn commit(mut self, destination: &Path, force: bool) -> io::Result<()> {
        if force {
            std::fs::rename(&self.path, destination)?;
        } else {
            // Linking fails atomically if another process creates the
            // destination after the early CLI existence check.
            std::fs::hard_link(&self.path, destination)?;
            std::fs::remove_file(&self.path)?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn destination_parent(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn sync_parent_directory(destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(destination_parent(destination))?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = destination;
        Ok(())
    }
}

fn resolve_format(format: Option<OutputFormat>, output: Option<&Path>) -> OutputFormat {
    format.unwrap_or_else(|| {
        if output
            .and_then(Path::extension)
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            OutputFormat::Pdf
        } else {
            OutputFormat::Epub
        }
    })
}

fn input_title(input: &Path) -> String {
    input
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Converted document")
        .to_string()
}

fn default_output_path(input: &Path, format: OutputFormat) -> PathBuf {
    let stem = input_title(input);
    let extension = match format {
        OutputFormat::Epub => "epub",
        OutputFormat::Pdf => "pdf",
    };
    input.with_file_name(format!("{stem}.paprika.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epub_is_the_default_output() {
        assert_eq!(
            default_output_path(Path::new("docs/paper.pdf"), OutputFormat::Epub),
            PathBuf::from("docs/paper.paprika.epub")
        );
    }

    #[test]
    fn infers_legacy_pdf_output_from_explicit_path() {
        assert_eq!(
            resolve_format(None, Some(Path::new("output.PDF"))),
            OutputFormat::Pdf
        );
        assert_eq!(
            resolve_format(None, Some(Path::new("output.epub"))),
            OutputFormat::Epub
        );
    }

    #[test]
    fn failed_forced_write_does_not_truncate_existing_destination() {
        let directory = TestDirectory::new("failed-write");
        let destination = directory.path.join("document.epub");
        std::fs::write(&destination, b"original document").unwrap();

        let result = write_destination_with(&destination, true, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::other("injected write failure"))
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"original document");
        assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 1);
    }

    #[test]
    fn no_force_write_does_not_clobber_a_racing_destination() {
        let directory = TestDirectory::new("no-clobber-race");
        let destination = directory.path.join("document.epub");

        let result = write_destination_with(&destination, false, |file| {
            file.write_all(b"our complete document")?;
            std::fs::write(&destination, b"racing writer")
        });

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&destination).unwrap(), b"racing writer");
        assert_eq!(std::fs::read_dir(&directory.path).unwrap().count(), 1);
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            for _ in 0..100 {
                let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "paprika-cli-{name}-{}-{sequence}",
                    std::process::id()
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("failed to create test directory: {error}"),
                }
            }
            panic!("failed to reserve a unique test directory");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
