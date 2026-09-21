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

/// Credentials and scope for the VLM-backed image-understanding pass.
///
/// Flattened into every subcommand that materialises a whole document, since the
/// pass runs after parsing completes. All three of `--ai-base-url`,
/// `--ai-api-key` and `--ai-model` must be present for the pass to run at all;
/// supplying none of them leaves output byte-identical to a build without them.
#[derive(Parser, Debug, Clone, Default)]
pub struct AiArgs {
    /// OpenAI-compatible endpoint base URL (without a trailing
    /// `/chat/completions`). Enables VLM image understanding together with
    /// --ai-api-key and --ai-model.
    #[arg(long, value_name = "URL")]
    pub ai_base_url: Option<String>,

    /// Bearer token for the AI endpoint.
    #[arg(long, value_name = "KEY", env = "UNPDF_AI_API_KEY")]
    pub ai_api_key: Option<String>,

    /// Model name to request from the AI endpoint.
    #[arg(long, value_name = "MODEL")]
    pub ai_model: Option<String>,

    /// Which parsed images to send to the model.
    #[arg(long, value_enum, default_value = "all")]
    pub ai_image_scope: AiImageScope,
}

/// Which images the VLM pass is invoked for.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, ValueEnum)]
pub enum AiImageScope {
    /// Every parsed image resource (quality first).
    #[default]
    All,
    /// Only pages the low-confidence OCR gate flagged as a text-free full-page
    /// scan (cost/latency first).
    LowConfidenceOnly,
}

impl From<AiImageScope> for unpdf::ImageScope {
    fn from(scope: AiImageScope) -> Self {
        match scope {
            AiImageScope::All => unpdf::ImageScope::All,
            AiImageScope::LowConfidenceOnly => unpdf::ImageScope::LowConfidencePagesOnly,
        }
    }
}

impl AiArgs {
    /// Builds the config the AI passes need, or `None` when the run is not
    /// configured for AI at all.
    ///
    /// Returns an error when the three required flags are supplied only in
    /// part: a half-configured endpoint is a typo, not a request to stay
    /// disabled, and silently skipping the pass would look like the model
    /// simply found nothing to say.
    fn to_config(&self) -> Result<Option<unpdf::AiConfig>, String> {
        match (
            self.ai_base_url.as_deref(),
            self.ai_api_key.as_deref(),
            self.ai_model.as_deref(),
        ) {
            (None, None, None) => Ok(None),
            (Some(base_url), Some(api_key), Some(model)) => {
                let mut config = unpdf::AiConfig::new(base_url, api_key, model);
                config.image_scope = self.ai_image_scope.into();
                Ok(Some(config))
            }
            (base_url, api_key, model) => {
                let missing: Vec<&str> = [
                    ("--ai-base-url", base_url.is_none()),
                    ("--ai-api-key (or UNPDF_AI_API_KEY)", api_key.is_none()),
                    ("--ai-model", model.is_none()),
                ]
                .into_iter()
                .filter_map(|(flag, absent)| absent.then_some(flag))
                .collect();
                Err(format!(
                    "incomplete AI configuration — also required: {}",
                    missing.join(", ")
                ))
            }
        }
    }
}

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

    /// Apply the markdown shape-refinement pass (table shape, list numbering,
    /// link/image paths, frontmatter, section anchors)
    #[arg(long)]
    pub refine: bool,

    #[command(flatten)]
    pub ai: AiArgs,

    /// Additionally run the rendered markdown through an AI refine pass,
    /// using the same credentials as the AI flags above
    #[arg(long)]
    pub ai_refine: bool,

    /// Output formats (comma-separated: md,txt,json)
    #[arg(long, value_delimiter = ',', default_value = "md")]
    pub formats: Vec<String>,

    /// Output all formats (MD + TXT + JSON)
    #[arg(long)]
    pub all: bool,

    /// Skip image extraction (images are extracted by default)
    #[arg(long)]
    pub no_images: bool,

    /// Keep a scan's OCR text layer even when it recognised nothing readable
    #[arg(long)]
    pub keep_ocr_text: bool,

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

    /// Insert HTML page boundary markers (<!-- page N -->)
    #[arg(long)]
    pub page_markers: bool,

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

    /// Apply the markdown shape-refinement pass (table shape, list numbering,
    /// link/image paths, frontmatter, section anchors)
    #[arg(long)]
    refine: bool,

    #[command(flatten)]
    ai: AiArgs,

    /// Additionally run the rendered markdown through an AI refine pass,
    /// using the same credentials as the AI flags above
    #[arg(long)]
    ai_refine: bool,

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

        /// Apply the markdown shape-refinement pass (table shape, list
        /// numbering, link/image paths, frontmatter, section anchors)
        #[arg(long)]
        refine: bool,

        #[command(flatten)]
        ai: AiArgs,

        /// Additionally run the rendered markdown through an AI refine pass,
        /// using the same credentials as the AI flags above
        #[arg(long)]
        ai_refine: bool,

        /// Maximum heading level (1-6)
        #[arg(long, default_value = "6")]
        max_heading: u8,

        /// Page range (e.g., "1-10", "1,3,5")
        #[arg(long)]
        pages: Option<String>,

        /// Insert HTML page boundary markers (<!-- page N -->)
        #[arg(long)]
        page_markers: bool,
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

        /// Accepted for API consistency with other subcommands; has no
        /// effect since plain text output is not markdown.
        #[arg(long)]
        refine: bool,

        #[command(flatten)]
        ai: AiArgs,

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

        #[command(flatten)]
        ai: AiArgs,
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
            refine,
            ai,
            ai_refine,
            max_heading,
            pages,
            page_markers,
        }) => cmd_markdown(
            &input,
            output.as_deref(),
            frontmatter,
            table_mode,
            cleanup,
            refine,
            &ai,
            ai_refine,
            max_heading,
            pages.as_deref(),
            page_markers,
            quiet,
        ),
        Some(Commands::Text {
            input,
            output,
            cleanup,
            refine,
            ai,
            pages,
        }) => {
            let _ = refine;
            cmd_text(
                &input,
                output.as_deref(),
                cleanup,
                &ai,
                pages.as_deref(),
                quiet,
            )
        }
        Some(Commands::Json {
            input,
            output,
            compact,
            ai,
        }) => cmd_json(&input, output.as_deref(), compact, &ai, quiet),
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
                    refine: cli.refine,
                    ai: cli.ai.clone(),
                    ai_refine: cli.ai_refine,
                    formats: vec!["md".to_string()],
                    all: false,
                    no_images: false,
                    keep_ocr_text: false,
                    image_dir: None,
                    min_image_size: 64,
                    window: None,
                    page_markers: false,
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
    if let Some(dir) = &image_dir {
        // Derived from the directory images are actually written to, not written out a
        // second time: `--image-dir` moves the files, and a hardcoded prefix would keep
        // pointing at `images/` where nothing had been written.
        render_opts = render_opts.with_image_prefix(image_link_prefix(&out_dir, dir));
    }
    if let Some(level) = args.cleanup {
        render_opts = render_opts.with_cleanup_preset(level.into());
    }
    if args.refine {
        render_opts = render_opts.with_refine();
    }
    if args.page_markers {
        render_opts = render_opts.with_page_markers(unpdf::PageMarkerStyle::Comment);
    }

    let ai_config = args.ai.to_config()?;
    if args.ai_refine && ai_config.is_none() {
        return Err("--ai-refine needs --ai-base-url, --ai-api-key and --ai-model".into());
    }
    if args.ai_refine {
        if let Some(config) = ai_config.clone() {
            render_opts = render_opts.with_ai_refine(config);
        }
    }

    // Open parser
    let mut parse_options = ParseOptions::new()
        .lenient()
        .with_ocr_suppression(!args.keep_ocr_text);
    if image_dir.is_some() {
        parse_options = parse_options.with_resources(true);
    }
    if let Some(config) = ai_config {
        // The AI passes run over an assembled document, which the streaming
        // pipeline below never produces. Buffer instead — a run paying for VLM
        // calls per page is not the run whose bottleneck is resident memory.
        // `min_image_dimension` moves onto the parse options here because the
        // streaming path takes it from `PageStreamOptions` instead.
        parse_options = parse_options
            .with_ai(config)
            .with_min_image_dimension(args.min_image_size);
        return convert_buffered(
            args,
            parse_options,
            render_opts,
            &out_dir,
            &formats,
            image_dir,
        );
    }
    let parser = PdfParser::open_with_options(&args.input, parse_options)?;

    // Set up writer
    let mut mfw =
        writer::MultiFormatWriter::new(&out_dir, &formats, render_opts, image_dir.clone())?;

    // Stream options
    let mut stream_opts = PageStreamOptions {
        extract_resources: image_dir.is_some(),
        min_image_dimension: args.min_image_size,
        suppress_low_confidence_ocr: !args.keep_ocr_text,
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
            ParseEvent::PageParsed(mut page) => {
                if let Err(e) = mfw.write_page(&mut page) {
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

    let summary = mfw.finish()?;
    pb.finish_with_message("Done");

    Ok(report_convert_result(
        args,
        &summary,
        image_dir.as_deref(),
        quality.as_ref(),
    ))
}

/// `convert` for a run with AI configured.
///
/// Same outputs as the streaming path, assembled from a whole document instead:
/// the AI passes need one, and the streaming pipeline never builds one (see
/// `cmd_convert`). The writer contract is identical, so pages are handed to it
/// in order exactly as the stream would have.
fn convert_buffered(
    args: &ConvertArgs,
    parse_options: ParseOptions,
    render_opts: RenderOptions,
    out_dir: &Path,
    formats: &[writer::OutputFormat],
    image_dir: Option<PathBuf>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut doc = parse_file_with_options(&args.input, parse_options)?;

    let mut mfw = writer::MultiFormatWriter::new(out_dir, formats, render_opts, image_dir.clone())?;

    let page_count = doc.pages.len() as u32;
    mfw.write_document_start(&doc.metadata, page_count)?;

    let pb = if args.quiet {
        ProgressBar::hidden()
    } else {
        let b = ProgressBar::new(page_count as u64);
        b.set_style(
            ProgressStyle::default_bar()
                .template("{bar:40.cyan/blue} {pos}/{len} pages ({eta})")
                .unwrap(),
        );
        b
    };

    for page in &mut doc.pages {
        mfw.write_page(page)?;
        pb.inc(1);
    }

    let summary = mfw.finish()?;
    pb.finish_with_message("Done");

    Ok(report_convert_result(
        args,
        &summary,
        image_dir.as_deref(),
        Some(&doc.extraction_quality),
    ))
}

/// Prints what `convert` produced and returns whether a quality warning fired,
/// which the caller turns into the process exit code.
fn report_convert_result(
    args: &ConvertArgs,
    summary: &writer::WriteSummary,
    image_dir: Option<&Path>,
    quality: Option<&unpdf::ExtractionQuality>,
) -> bool {
    if !args.quiet {
        for path in [&summary.md_path, &summary.txt_path, &summary.json_path]
            .into_iter()
            .flatten()
        {
            println!("{} {}", "✓".green(), path.display());
        }
        if summary.image_count > 0 {
            let img_dir = image_dir.unwrap_or_else(|| Path::new("images"));
            println!(
                "{} {} image{} → {}",
                "✓".green(),
                summary.image_count,
                if summary.image_count == 1 { "" } else { "s" },
                img_dir.display()
            );
        }
        if summary.word_count > 0 {
            println!("{} {} words", "✓".green(), summary.word_count);
        }
    }

    let warning = quality.and_then(|q| q.warning_message());
    if let Some(warning) = &warning {
        if !args.quiet {
            eprintln!("{}: {}", "Warning".yellow().bold(), warning);
        }
    }
    warning.is_some()
}

#[allow(clippy::too_many_arguments)]
fn cmd_markdown(
    input: &Path,
    output: Option<&Path>,
    frontmatter: bool,
    table_mode: TableMode,
    cleanup: Option<CleanupLevel>,
    refine: bool,
    ai: &AiArgs,
    ai_refine: bool,
    max_heading: u8,
    pages: Option<&str>,
    page_markers: bool,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    let ai_config = ai.to_config()?;
    if ai_refine && ai_config.is_none() {
        return Err("--ai-refine needs --ai-base-url, --ai-api-key and --ai-model".into());
    }

    // Use lenient mode to continue even if some text extraction fails
    let mut options = ParseOptions::new()
        .lenient()
        .with_pages(page_selection.clone());
    if let Some(config) = ai_config.clone() {
        options = options.with_ai(config);
    }
    let doc = parse_file_with_options(input, options)?;
    let had_warnings = check_quality(&doc, quiet);

    let mut render_options = RenderOptions::new()
        .with_frontmatter(frontmatter)
        .with_table_fallback(table_mode.into())
        .with_max_heading(max_heading)
        .with_pages(page_selection);

    if page_markers {
        render_options = render_options.with_page_markers(unpdf::PageMarkerStyle::Comment);
    }

    if let Some(level) = cleanup {
        render_options = render_options.with_cleanup_preset(level.into());
    }
    if refine {
        render_options = render_options.with_refine();
    }
    if ai_refine {
        // Guarded above: `ai_refine` without a config is rejected before parsing.
        if let Some(config) = ai_config {
            render_options = render_options.with_ai_refine(config);
        }
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
    ai: &AiArgs,
    pages: Option<&str>,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    // Use lenient mode to continue even if some text extraction fails
    let mut options = ParseOptions::new().lenient().with_pages(page_selection);
    if let Some(config) = ai.to_config()? {
        options = options.with_ai(config);
    }
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
    ai: &AiArgs,
    quiet: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Use lenient mode to continue even if some text extraction fails
    let mut options = ParseOptions::new().lenient();
    if let Some(config) = ai.to_config()? {
        options = options.with_ai(config);
    }
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
    // Report page loss on the Pages line itself, not only through the quality warning:
    // `--quiet` silences warnings, and a diagnostic command must not hide a short page
    // set just because the caller asked for less noise.
    let q = &doc.extraction_quality;
    match (q.pages_incomplete, q.declared_page_count) {
        (true, Some(declared)) => println!(
            "{}: {} {}",
            "Pages".bold(),
            doc.metadata.page_count,
            format!("(incomplete — document declares {})", declared).yellow()
        ),
        (true, None) => println!(
            "{}: {} {}",
            "Pages".bold(),
            doc.metadata.page_count,
            "(incomplete — page structure damaged)".yellow()
        ),
        _ => println!("{}: {}", "Pages".bold(), doc.metadata.page_count),
    }
    // Same reasoning as the Pages line above: text the decoder could not read is
    // content missing from the output, and `--quiet` would silence the warning that
    // otherwise carries it. Printed only when non-zero — an intact document should
    // not have to read a line saying nothing was lost.
    if q.suppressed_text_runs > 0 {
        println!(
            "{}: {}",
            "Text runs".bold(),
            format!(
                "{} unreadable, dropped (fonts' character codes unresolved)",
                q.suppressed_text_runs
            )
            .yellow()
        );
    }
    // Same reasoning again: a content stream that could not be decoded is content missing
    // from the output, and a page whose only stream failed looks blank otherwise.
    if q.undecodable_content_streams > 0 {
        println!(
            "{}: {}",
            "Content streams".bold(),
            format!("{} undecodable, left out", q.undecodable_content_streams).yellow()
        );
    }
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

/// The markdown link prefix for images, derived from where they are actually written.
///
/// A link is resolved relative to the document that carries it, so an image directory
/// inside the output directory becomes a relative prefix (`images/`). One outside it --
/// `--image-dir` pointing elsewhere -- is emitted as given, which is the only form that
/// can still resolve. Separators are normalised to `/`: a Windows path is a valid link
/// destination only in that form.
fn image_link_prefix(out_dir: &Path, image_dir: &Path) -> String {
    let rel = image_dir.strip_prefix(out_dir).unwrap_or(image_dir);
    let text = rel.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        String::new()
    } else if text.ends_with('/') {
        text
    } else {
        format!("{}/", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Catches flag collisions introduced by flattening `AiArgs` into several
    /// subcommands, which clap only reports at runtime otherwise.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// The default layout: images land in `<out>/images`, so the link carries exactly
    /// that one segment and nothing more.
    #[test]
    fn image_link_prefix_is_relative_for_the_default_layout() {
        let out = Path::new("/tmp/doc_output");
        assert_eq!(image_link_prefix(out, &out.join("images")), "images/");
    }

    /// `--image-dir` pointing deeper inside the output directory keeps the whole
    /// relative path -- the link is resolved from the markdown file, not from the leaf.
    #[test]
    fn image_link_prefix_keeps_nested_relative_paths() {
        let out = Path::new("/tmp/doc_output");
        assert_eq!(
            image_link_prefix(out, &out.join("assets").join("img")),
            "assets/img/"
        );
    }

    /// `--image-dir` pointing outside the output directory cannot be expressed relative
    /// to it, so the path is emitted as given. A hardcoded `images/` here was the defect:
    /// the files went one place and the links pointed at another.
    #[test]
    fn image_link_prefix_falls_back_to_the_path_as_given_when_outside() {
        let out = Path::new("/tmp/doc_output");
        assert_eq!(
            image_link_prefix(out, Path::new("/var/shared/pics")),
            "/var/shared/pics/"
        );
    }

    /// Images written beside the markdown need no prefix at all.
    #[test]
    fn image_link_prefix_is_empty_when_images_sit_in_the_output_root() {
        let out = Path::new("/tmp/doc_output");
        assert_eq!(image_link_prefix(out, out), "");
    }

    fn args(base_url: Option<&str>, api_key: Option<&str>, model: Option<&str>) -> AiArgs {
        AiArgs {
            ai_base_url: base_url.map(String::from),
            ai_api_key: api_key.map(String::from),
            ai_model: model.map(String::from),
            ai_image_scope: AiImageScope::All,
        }
    }

    #[test]
    fn no_ai_flags_yields_no_config() {
        assert!(args(None, None, None).to_config().unwrap().is_none());
    }

    #[test]
    fn all_three_flags_yield_a_config() {
        let config = args(Some("http://localhost"), Some("k"), Some("m"))
            .to_config()
            .unwrap()
            .expect("all three supplied");
        assert_eq!(config.base_url, "http://localhost");
        assert_eq!(config.api_key, "k");
        assert_eq!(config.model, "m");
        assert_eq!(config.image_scope, unpdf::ImageScope::All);
    }

    #[test]
    fn image_scope_maps_onto_the_library_enum() {
        let mut a = args(Some("u"), Some("k"), Some("m"));
        a.ai_image_scope = AiImageScope::LowConfidenceOnly;
        let config = a.to_config().unwrap().unwrap();
        assert_eq!(
            config.image_scope,
            unpdf::ImageScope::LowConfidencePagesOnly
        );
    }

    /// A half-supplied endpoint is a typo, not a request to stay disabled —
    /// skipping the pass silently would look like the model found nothing.
    #[test]
    fn partial_config_is_an_error_naming_what_is_missing() {
        let err = args(None, None, Some("m")).to_config().unwrap_err();
        assert!(err.contains("--ai-base-url"), "{err}");
        assert!(err.contains("--ai-api-key"), "{err}");
        assert!(!err.contains("--ai-model"), "{err}");

        let err = args(Some("u"), Some("k"), None).to_config().unwrap_err();
        assert!(err.contains("--ai-model"), "{err}");
        assert!(!err.contains("--ai-base-url"), "{err}");
    }

    fn subcommand_args(name: &str) -> Vec<String> {
        Cli::command()
            .get_subcommands()
            .find(|c| c.get_name() == name)
            .unwrap_or_else(|| panic!("{name} subcommand"))
            .get_arguments()
            .map(|a| a.get_id().to_string())
            .collect()
    }

    /// `--ai-refine` rewrites markdown, so it belongs exactly where `--refine`
    /// has an effect — not on `text` (whose `--refine` is already a documented
    /// no-op) and not on `json`.
    #[test]
    fn ai_refine_is_scoped_to_markdown_rendering_commands() {
        let has_ai_refine = |name: &str| subcommand_args(name).iter().any(|a| a == "ai_refine");
        assert!(has_ai_refine("markdown"));
        assert!(has_ai_refine("convert"), "convert renders extract.md");
        assert!(!has_ai_refine("text"));
        assert!(!has_ai_refine("json"));
    }

    /// Parse-time AI flags belong on every command that assembles a Document —
    /// `json` included (its output carries both the image understanding and the
    /// `ai_fallback_count` statistic), and `convert` included, which switches
    /// from streaming to a buffered parse precisely so it can offer them.
    #[test]
    fn parse_time_ai_flags_reach_every_command_that_assembles_a_document() {
        for name in ["convert", "markdown", "text", "json"] {
            let args = subcommand_args(name);
            for flag in ["ai_base_url", "ai_api_key", "ai_model", "ai_image_scope"] {
                assert!(
                    args.iter().any(|a| a == flag),
                    "{name} is missing --{}",
                    flag.replace('_', "-")
                );
            }
        }
    }

    /// The commands that cannot use a parsed document's contents must not grow
    /// the flags: `info` reads metadata only, `extract` writes image bytes.
    #[test]
    fn ai_flags_stay_off_commands_that_cannot_use_them() {
        for name in ["info", "extract"] {
            assert!(
                !subcommand_args(name).iter().any(|a| a.starts_with("ai_")),
                "{name} has no use for the AI passes"
            );
        }
    }
}
