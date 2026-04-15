//! unpdf CLI - PDF content extraction tool

mod update;
mod writer;

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use unpdf::{
    parse_file_with_options, CleanupPreset, JsonFormat, PageSelection, ParseOptions, RenderOptions,
};
use unpdf::{PageStreamOptions, ParseEvent, PdfParser};

/// Arguments for the `convert` subcommand.
#[derive(Parser, Debug)]
pub struct ConvertArgs {
    /// Input PDF file
    #[arg(value_name = "FILE")]
    pub input: PathBuf,

    /// Output directory
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Text cleanup preset
    #[arg(long, value_enum)]
    pub cleanup: Option<CleanupLevel>,

    /// Output formats (comma-separated: md,txt,json)
    #[arg(long, value_delimiter = ',', default_value = "md")]
    pub formats: Vec<String>,

    /// Output all formats (MD + TXT + JSON)
    #[arg(long)]
    pub all: bool,

    /// Skip image extraction (images are extracted by default)
    #[arg(long)]
    pub no_images: bool,

    /// Directory for extracted images (defaults to `<out>/images`)
    #[arg(long, value_name = "DIR")]
    pub image_dir: Option<PathBuf>,

    /// Minimum pixel dimension for extracted images. Smaller images are
    /// dropped as decorative (logos, bullets, rules). 0 keeps all.
    #[arg(long, value_name = "PX", default_value = "64")]
    pub min_image_size: u32,

    /// Override streaming window size (pages in-flight)
    #[arg(long, value_name = "N")]
    pub window: Option<usize>,

    /// Suppress warning messages
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser)]
#[command(name = "unpdf")]
#[command(author = "iyulab")]
#[command(version)]
#[command(about = "Extract PDF content to Markdown, text, and JSON", long_about = None)]
struct Cli {
    /// Input PDF file
    #[arg(value_name = "FILE")]
    input: Option<PathBuf>,

    /// Output directory
    #[arg(value_name = "OUTPUT")]
    output: Option<PathBuf>,

    /// Text cleanup preset
    #[arg(long, value_enum)]
    cleanup: Option<CleanupLevel>,

    /// Suppress warning messages
    #[arg(short, long)]
    quiet: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert PDF to Markdown, text, and/or JSON (streaming pipeline)
    Convert(ConvertArgs),

    /// Convert PDF to Markdown
    #[command(alias = "md")]
    Markdown {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Output file (stdout if not specified)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Include YAML frontmatter
        #[arg(short, long)]
        frontmatter: bool,

        /// Table rendering mode
        #[arg(long, value_enum, default_value = "markdown")]
        table_mode: TableMode,

        /// Text cleanup preset
        #[arg(long, value_enum)]
        cleanup: Option<CleanupLevel>,

        /// Maximum heading level (1-6)
        #[arg(long, default_value = "6")]
        max_heading: u8,

        /// Page range (e.g., "1-10", "1,3,5")
        #[arg(long)]
        pages: Option<String>,
    },

    /// Convert PDF to plain text
    Text {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Output file (stdout if not specified)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Text cleanup preset
        #[arg(long, value_enum)]
        cleanup: Option<CleanupLevel>,

        /// Page range (e.g., "1-10", "1,3,5")
        #[arg(long)]
        pages: Option<String>,
    },

    /// Convert PDF to JSON
    Json {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Output file (stdout if not specified)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Output compact JSON
        #[arg(long)]
        compact: bool,
    },

    /// Show document information
    Info {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,
    },

    /// Extract images from PDF
    Extract {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Output directory
        #[arg(short, long, value_name = "DIR")]
        output: Option<PathBuf>,

        /// Page range (e.g., "1-10", "1,3,5")
        #[arg(long)]
        pages: Option<String>,
    },

    /// Self-update to latest version
    Update {
        /// Only check for updates, don't install
        #[arg(long)]
        check: bool,

        /// Force reinstall even if up-to-date
        #[arg(long)]
        force: bool,
    },

    /// Show version information
    Version,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum CleanupLevel {
    /// Minimal cleanup (Unicode normalization only)
    Minimal,
    /// Standard cleanup (default)
    Standard,
    /// Aggressive cleanup (for LLM training)
    Aggressive,
}

impl From<CleanupLevel> for CleanupPreset {
    fn from(level: CleanupLevel) -> Self {
        match level {
            CleanupLevel::Minimal => CleanupPreset::Minimal,
            CleanupLevel::Standard => CleanupPreset::Standard,
            CleanupLevel::Aggressive => CleanupPreset::Aggressive,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TableMode {
    /// Standard Markdown tables
    Markdown,
    /// HTML tables for complex layouts
    Html,
    /// ASCII art tables
    Ascii,
}

impl From<TableMode> for unpdf::TableFallback {
    fn from(mode: TableMode) -> Self {
        match mode {
            TableMode::Markdown => unpdf::TableFallback::Markdown,
            TableMode::Html => unpdf::TableFallback::Html,
            TableMode::Ascii => unpdf::TableFallback::Ascii,
        }
    }
}

/// Check extraction quality and print warnings to stderr.
/// Returns true if quality warnings were emitted.
fn check_quality(doc: &unpdf::Document, quiet: bool) -> bool {
    if quiet {
        return false;
    }
    if let Some(warning) = doc.extraction_quality.warning_message() {
        eprintln!("{}: {}", "Warning".yellow().bold(), warning);
        return true;
    }
    false
}

/// Check if we should perform background update check.
/// Skip for update/version commands to avoid redundant checks.
fn should_check_update(cli: &Cli) -> bool {
    !matches!(
        &cli.command,
        Some(Commands::Update { .. }) | Some(Commands::Version)
    )
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    // Start background update check (except for update/version commands)
    let update_rx = if should_check_update(&cli) {
        Some(update::check_update_async())
    } else {
        None
    };

    let quiet = cli.quiet;

    let result = match cli.command {
        Some(Commands::Convert(mut args)) => {
            // Top-level --quiet propagates into ConvertArgs
            if quiet {
                args.quiet = true;
            }
            cmd_convert(&args)
        }
        Some(Commands::Markdown {
            input,
            output,
            frontmatter,
            table_mode,
            cleanup,
            max_heading,
            pages,
        }) => cmd_markdown(
            &input,
            output.as_deref(),
            frontmatter,
            table_mode,
            cleanup,
            max_heading,
            pages.as_deref(),
            quiet,
        ),
        Some(Commands::Text {
            input,
            output,
            cleanup,
            pages,
        }) => cmd_text(&input, output.as_deref(), cleanup, pages.as_deref(), quiet),
        Some(Commands::Json {
            input,
            output,
            compact,
        }) => cmd_json(&input, output.as_deref(), compact, quiet),
        Some(Commands::Info { input }) => cmd_info(&input, quiet),
        Some(Commands::Extract {
            input,
            output,
            pages,
        }) => cmd_extract(&input, output.as_deref(), pages.as_deref(), quiet),
        Some(Commands::Update { check, force }) => {
            if let Err(e) = update::run_update(check, force) {
                eprintln!("{}: {}", "Error".red().bold(), e);
                std::process::exit(1);
            }
            Ok(false)
        }
        Some(Commands::Version) => {
            cmd_version();
            Ok(false)
        }
        None => {
            // Default behavior: convert if input is provided
            if let Some(input) = cli.input {
                let args = ConvertArgs {
                    input,
                    output: cli.output,
                    cleanup: cli.cleanup,
                    formats: vec!["md".to_string()],
                    all: false,
                    no_images: false,
                    image_dir: None,
                    min_image_size: 64,
                    window: None,
                    quiet,
                };
                cmd_convert(&args)
            } else {
                println!("{}", "Usage: unpdf <FILE> [OUTPUT]".yellow());
                println!("       unpdf --help for more information");
                Ok(false)
            }
        }
    };

    // Check for update result and show notification if available
    if let Some(rx) = update_rx {
        if let Some(update_result) = update::try_get_update_result(&rx) {
            update::print_update_notification(&update_result);
        }
    }

    match result {
        Ok(had_warnings) => {
            if had_warnings {
                std::process::exit(2);
            }
        }
        Err(e) => {
            eprintln!("{}: {}", "Error".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn cmd_convert(args: &ConvertArgs) -> Result<bool, Box<dyn std::error::Error>> {
    use std::ops::ControlFlow;

    let out_dir = args.output.clone().unwrap_or_else(|| {
        let stem = args.input.file_stem().unwrap_or_default().to_string_lossy();
        PathBuf::from(format!("{}_output", stem))
    });
    fs::create_dir_all(&out_dir)?;

    // Determine output formats
    let formats: Vec<writer::OutputFormat> = if args.all {
        vec![
            writer::OutputFormat::Markdown,
            writer::OutputFormat::Text,
            writer::OutputFormat::Json,
        ]
    } else {
        let mut v: Vec<_> = args
            .formats
            .iter()
            .filter_map(|s| match s.as_str() {
                "md" | "markdown" => Some(writer::OutputFormat::Markdown),
                "txt" | "text" => Some(writer::OutputFormat::Text),
                "json" => Some(writer::OutputFormat::Json),
                other => {
                    eprintln!("warning: unknown format: {}", other);
                    None
                }
            })
            .collect();
        if v.is_empty() {
            v.push(writer::OutputFormat::Markdown);
        }
        v
    };

    // Image extraction configuration — 기본 on. `--no-images` 로 옵트아웃.
    // `--image-dir` 지정 시 그 경로가 우선, 없으면 `<out>/images` 사용.
    // 디렉토리는 첫 이미지가 실제로 쓰일 때만 생성 (이미지 없는 PDF 에서
    // 빈 폴더가 남는 것 방지) — MultiFormatWriter 내부에서 처리.
    let image_dir: Option<PathBuf> = if args.no_images {
        None
    } else {
        Some(
            args.image_dir
                .clone()
                .unwrap_or_else(|| out_dir.join("images")),
        )
    };

    // Build render options
    let mut render_opts = RenderOptions::new().with_frontmatter(true);
    if image_dir.is_some() {
        render_opts = render_opts.with_image_prefix("images/");
    }
    if let Some(level) = args.cleanup {
        render_opts = render_opts.with_cleanup_preset(level.into());
    }

    // Open parser
    let mut parse_options = ParseOptions::new().lenient();
    if image_dir.is_some() {
        parse_options = parse_options.with_resources(true);
    }
    let parser = PdfParser::open_with_options(&args.input, parse_options)?;

    // Set up writer
    let mut mfw =
        writer::MultiFormatWriter::new(&out_dir, &formats, render_opts, image_dir.clone())?;

    // Stream options
    let mut stream_opts = PageStreamOptions {
        extract_resources: image_dir.is_some(),
        min_image_dimension: args.min_image_size,
        ..PageStreamOptions::default()
    };
    if let Some(w) = args.window {
        stream_opts.window_size = w.max(1);
    }

    // Progress bar
    let total_pages = parser.page_count();
    let pb = if args.quiet {
        ProgressBar::hidden()
    } else {
        let b = ProgressBar::new(total_pages as u64);
        b.set_style(
            ProgressStyle::default_bar()
                .template("{bar:40.cyan/blue} {pos}/{len} pages ({eta})")
                .unwrap(),
        );
        b
    };

    let mut quality = None;
    let mut write_err: Option<String> = None;

    parser.for_each_page(stream_opts, |ev| {
        match ev {
            ParseEvent::DocumentStart {
                metadata,
                page_count,
                ..
            } => {
                if let Err(e) = mfw.write_document_start(&metadata, page_count) {
                    write_err = Some(format!("document start: {}", e));
                    return ControlFlow::Break(());
                }
            }
            ParseEvent::PageParsed(page) => {
                if let Err(e) = mfw.write_page(&page) {
                    write_err = Some(format!("page {}: {}", page.number, e));
                    return ControlFlow::Break(());
                }
                pb.inc(1);
            }
            ParseEvent::PageFailed { page, error } => {
                eprintln!("page {} failed: {}", page, error);
                pb.inc(1);
            }
            ParseEvent::DocumentEnd { quality: q } => {
                quality = Some(q);
            }
            ParseEvent::Progress { .. } => {}
        }
        ControlFlow::Continue(())
    })?;

    if let Some(e) = write_err {
        return Err(e.into());
    }

    let img_count = mfw.image_count();
    mfw.finish()?;
    pb.finish_with_message("Done");

    if !args.quiet && img_count > 0 {
        println!(
            "{} {} image{} extracted to images/",
            "✓".green(),
            img_count,
            if img_count == 1 { "" } else { "s" }
        );
    }

    let had_warnings = quality
        .as_ref()
        .map(|q| q.warning_message().is_some())
        .unwrap_or(false);
    Ok(had_warnings)
}

#[allow(clippy::too_many_arguments)]
fn cmd_markdown(
    input: &Path,
    output: Option<&Path>,
    frontmatter: bool,
    table_mode: TableMode,
    cleanup: Option<CleanupLevel>,
    max_heading: u8,
    pages: Option<&str>,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    // Use lenient mode to continue even if some text extraction fails
    let options = ParseOptions::new()
        .lenient()
        .with_pages(page_selection.clone());
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let mut render_options = RenderOptions::new()
        .with_frontmatter(frontmatter)
        .with_table_fallback(table_mode.into())
        .with_max_heading(max_heading)
        .with_pages(page_selection);

    if let Some(level) = cleanup {
        render_options = render_options.with_cleanup_preset(level.into());
    }

    let markdown = unpdf::render::to_markdown(&doc, &render_options)?;

    if let Some(path) = output {
        fs::write(path, &markdown)?;
        println!("{} {}", "Saved to".green(), path.display());
    } else {
        println!("{}", markdown);
    }

    Ok(had_warnings)
}

fn cmd_text(
    input: &Path,
    output: Option<&Path>,
    cleanup: Option<CleanupLevel>,
    pages: Option<&str>,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    // Use lenient mode to continue even if some text extraction fails
    let options = ParseOptions::new().lenient().with_pages(page_selection);
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let mut render_options = RenderOptions::new();
    if let Some(level) = cleanup {
        render_options = render_options.with_cleanup_preset(level.into());
    }

    let text = unpdf::render::to_text(&doc, &render_options)?;

    if let Some(path) = output {
        fs::write(path, &text)?;
        println!("{} {}", "Saved to".green(), path.display());
    } else {
        println!("{}", text);
    }

    Ok(had_warnings)
}

fn cmd_json(
    input: &Path,
    output: Option<&Path>,
    compact: bool,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Use lenient mode to continue even if some text extraction fails
    let options = ParseOptions::new().lenient();
    let doc = unpdf::parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let format = if compact {
        JsonFormat::Compact
    } else {
        JsonFormat::Pretty
    };

    let json = unpdf::render::to_json(&doc, format)?;

    if let Some(path) = output {
        fs::write(path, &json)?;
        println!("{} {}", "Saved to".green(), path.display());
    } else {
        println!("{}", json);
    }

    Ok(had_warnings)
}

fn cmd_info(input: &Path, quiet: bool) -> Result<bool, Box<dyn std::error::Error>> {
    // Use lenient mode for info command - we want to show metadata even if text extraction fails
    let options = ParseOptions::new().lenient();
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    println!("{}", "Document Information".cyan().bold());
    println!("{}", "─".repeat(40).dimmed());

    println!("{}: {}", "File".bold(), input.display());
    println!("{}: PDF {}", "Format".bold(), doc.metadata.pdf_version);
    println!("{}: {}", "Pages".bold(), doc.metadata.page_count);
    println!(
        "{}: {}",
        "Encrypted".bold(),
        if doc.metadata.encrypted { "Yes" } else { "No" }
    );

    if let Some(ref title) = doc.metadata.title {
        println!("{}: {}", "Title".bold(), title);
    }
    if let Some(ref author) = doc.metadata.author {
        println!("{}: {}", "Author".bold(), author);
    }
    if let Some(ref creator) = doc.metadata.creator {
        println!("{}: {}", "Creator".bold(), creator);
    }
    if let Some(ref producer) = doc.metadata.producer {
        println!("{}: {}", "Producer".bold(), producer);
    }
    if let Some(ref created) = doc.metadata.created {
        println!("{}: {}", "Created".bold(), created);
    }
    if let Some(ref modified) = doc.metadata.modified {
        println!("{}: {}", "Modified".bold(), modified);
    }

    println!();
    println!("{}", "Content Statistics".cyan().bold());
    println!("{}", "─".repeat(40).dimmed());

    let text = doc.plain_text();
    let words: usize = text.split_whitespace().count();
    let chars = text.len();
    let images = doc.resources.values().filter(|r| r.is_image()).count();

    println!("{}: {}", "Words".bold(), words);
    println!("{}: {}", "Characters".bold(), chars);
    println!("{}: {}", "Images".bold(), images);

    if let Some(ref outline) = doc.outline {
        println!("{}: {}", "Bookmarks".bold(), outline.total_items());
    }

    Ok(had_warnings)
}

fn cmd_extract(
    input: &Path,
    output: Option<&Path>,
    pages: Option<&str>,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    // Use lenient mode to continue even if some text extraction fails
    let options = ParseOptions::new().lenient().with_pages(page_selection);
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let output_dir = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&output_dir)?;

    let mut count = 0;
    for (id, resource) in &doc.resources {
        if resource.is_image() {
            let filename = resource.suggested_filename(id);
            let path = output_dir.join(&filename);
            fs::write(&path, &resource.data)?;
            println!("{} {}", "Extracted".green(), filename);
            count += 1;
        }
    }

    println!("\n{} {} images extracted", "Done!".green().bold(), count);

    Ok(had_warnings)
}

fn cmd_version() {
    println!("{} {}", "unpdf".cyan().bold(), env!("CARGO_PKG_VERSION"));
    println!("PDF content extraction tool");
    println!();
    println!("Repository: {}", "https://github.com/iyulab/unpdf".dimmed());
    println!("License: MIT");
}
