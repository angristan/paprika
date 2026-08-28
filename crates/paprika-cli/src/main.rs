use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use paprika_core::{Mode, OptimizationOptions};
use paprika_pdf::optimize_pdf;

#[derive(Debug, Parser)]
#[command(
    name = "paprika",
    version,
    about = "Reflow PDF pages for small screens",
    long_about = "Paprika trims margins, detects columns, and graphically reflows PDF pages into a raster PDF sized for an e-reader or phone. Documents never leave this machine."
)]
struct Arguments {
    /// Source PDF.
    input: PathBuf,

    /// Destination PDF. Defaults to <input>.paprika.pdf.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Layout strategy.
    #[arg(long, value_enum, default_value_t = CliMode::Reflow)]
    mode: CliMode,

    /// Output width in pixels.
    #[arg(long, default_value_t = 758)]
    width: u32,

    /// Output height in pixels.
    #[arg(long, default_value_t = 1024)]
    height: u32,

    /// Output resolution in dots per inch.
    #[arg(long, default_value_t = 167)]
    dpi: u32,

    /// Resolution used to render source pages.
    #[arg(long, default_value_t = 144)]
    source_dpi: u32,

    /// Output margin in pixels.
    #[arg(long, default_value_t = 24)]
    margin: u32,

    /// Approximate graphical text size in points in reflow mode.
    #[arg(long, default_value_t = 12.0)]
    font_size: f32,

    /// Grayscale value below which pixels count as content.
    #[arg(long, default_value_t = 245)]
    threshold: u8,

    /// Maximum detected columns (1 or 2).
    #[arg(long, default_value_t = 2)]
    columns: u8,

    /// Replace an existing destination file.
    #[arg(long)]
    force: bool,

    /// Suppress the conversion summary.
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CliMode {
    #[default]
    Reflow,
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
    let output = arguments
        .output
        .clone()
        .unwrap_or_else(|| default_output_path(&arguments.input));

    if arguments.input == output {
        bail!("input and output paths must differ");
    }
    if output.exists() && !arguments.force {
        bail!(
            "{} already exists; choose another output or pass --force",
            output.display()
        );
    }

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

    let started = Instant::now();
    let input = std::fs::read(&arguments.input)
        .with_context(|| format!("could not read {}", arguments.input.display()))?;
    let result = optimize_pdf(&input, options).context("PDF conversion failed")?;
    std::fs::write(&output, &result.bytes)
        .with_context(|| format!("could not write {}", output.display()))?;

    if !arguments.quiet {
        eprintln!(
            "{} source page(s) → {} output page(s), {} bytes in {:.2}s\n{}",
            result.source_pages,
            result.output_pages,
            result.bytes.len(),
            started.elapsed().as_secs_f32(),
            output.display()
        );
    }
    Ok(())
}

fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    input.with_file_name(format!("{stem}.paprika.pdf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_name_is_adjacent_to_input() {
        assert_eq!(
            default_output_path(Path::new("docs/paper.pdf")),
            PathBuf::from("docs/paper.paprika.pdf")
        );
    }
}
