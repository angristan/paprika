use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use paprika_core::{Mode, OptimizationOptions};
use paprika_epub::{EpubOptions, convert_pdf_to_epub};
use paprika_pdf::optimize_pdf;

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
            let result = convert_pdf_to_epub(
                &input,
                EpubOptions {
                    title,
                    language: arguments.language,
                    include_images: !arguments.no_images,
                    ..Default::default()
                },
            )
            .context("EPUB conversion failed")?;
            std::fs::write(&output, &result.bytes)
                .with_context(|| format!("could not write {}", output.display()))?;
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
            let result = optimize_pdf(&input, options).context("raster PDF conversion failed")?;
            std::fs::write(&output, &result.bytes)
                .with_context(|| format!("could not write {}", output.display()))?;
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
}
