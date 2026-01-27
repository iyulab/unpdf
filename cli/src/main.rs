//! unpdf CLI - PDF content extraction tool

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use unpdf::{
    parse_file_with_options, CleanupPreset, JsonFormat, PageSelection, ParseOptions, RenderOptions,
};

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

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert PDF to all formats (Markdown, text, JSON)
    Convert {
        /// Input PDF file
        #[arg(value_name = "FILE")]
        input: PathBuf,

        /// Output directory
        #[arg(short, long, value_name = "DIR")]
        output: Option<PathBuf>,

        /// Text cleanup preset
        #[arg(long, value_enum)]
        cleanup: Option<CleanupLevel>,
    },

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

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum CleanupLevel {
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

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    let result = match cli.command {
        Some(Commands::Convert {
            input,
            output,
            cleanup,
        }) => cmd_convert(&input, output.as_deref(), cleanup),
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
        ),
        Some(Commands::Text {
            input,
            output,
            cleanup,
            pages,
        }) => cmd_text(&input, output.as_deref(), cleanup, pages.as_deref()),
        Some(Commands::Json {
            input,
            output,
            compact,
        }) => cmd_json(&input, output.as_deref(), compact),
        Some(Commands::Info { input }) => cmd_info(&input),
        Some(Commands::Extract {
            input,
            output,
            pages,
        }) => cmd_extract(&input, output.as_deref(), pages.as_deref()),
        Some(Commands::Update { check, force }) => cmd_update(check, force),
        Some(Commands::Version) => {
            cmd_version();
            Ok(())
        }
        None => {
            // Default behavior: convert if input is provided
            if let Some(input) = cli.input {
                cmd_convert(&input, cli.output.as_deref(), cli.cleanup)
            } else {
                println!("{}", "Usage: unpdf <FILE> [OUTPUT]".yellow());
                println!("       unpdf --help for more information");
                Ok(())
            }
        }
    };

    if let Err(e) = result {
        eprintln!("{}: {}", "Error".red().bold(), e);
        std::process::exit(1);
    }
}

fn cmd_convert(
    input: &Path,
    output: Option<&Path>,
    cleanup: Option<CleanupLevel>,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = output.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        PathBuf::from(format!("{}_output", stem))
    });

    fs::create_dir_all(&output_dir)?;

    let pb = ProgressBar::new(4);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    // Parse document with lenient mode to handle malformed PDFs
    pb.set_message("Parsing PDF...");
    let options = ParseOptions::new().lenient();
    let doc = parse_file_with_options(input, options)?;
    pb.inc(1);

    // Build render options
    let mut render_options = RenderOptions::new()
        .with_frontmatter(true)
        .with_image_dir(output_dir.join("images"))
        .with_image_prefix("images/");

    if let Some(level) = cleanup {
        render_options = render_options.with_cleanup_preset(level.into());
    }

    // Extract images
    pb.set_message("Extracting images...");
    let images_dir = output_dir.join("images");
    fs::create_dir_all(&images_dir)?;
    for (id, resource) in &doc.resources {
        if resource.is_image() {
            let filename = resource.suggested_filename(id);
            let path = images_dir.join(&filename);
            fs::write(&path, &resource.data)?;
        }
    }
    pb.inc(1);

    // Generate Markdown
    pb.set_message("Generating Markdown...");
    let markdown = unpdf::render::to_markdown(&doc, &render_options)?;
    fs::write(output_dir.join("extract.md"), &markdown)?;
    pb.inc(1);

    // Generate text
    pb.set_message("Generating text...");
    let text = unpdf::render::to_text(&doc, &render_options)?;
    fs::write(output_dir.join("extract.txt"), &text)?;

    // Generate JSON
    let json = unpdf::render::to_json(&doc, JsonFormat::Pretty)?;
    fs::write(output_dir.join("content.json"), &json)?;
    pb.inc(1);

    pb.finish_with_message("Done!");

    println!("\n{}", "Output files:".green().bold());
    println!("  {} extract.md", "├─".dimmed());
    println!("  {} extract.txt", "├─".dimmed());
    println!("  {} content.json", "├─".dimmed());
    println!("  {} images/", "└─".dimmed());

    Ok(())
}

fn cmd_markdown(
    input: &Path,
    output: Option<&Path>,
    frontmatter: bool,
    table_mode: TableMode,
    cleanup: Option<CleanupLevel>,
    max_heading: u8,
    pages: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
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

    Ok(())
}

fn cmd_text(
    input: &Path,
    output: Option<&Path>,
    cleanup: Option<CleanupLevel>,
    pages: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    // Use lenient mode to continue even if some text extraction fails
    let options = ParseOptions::new().lenient().with_pages(page_selection);
    let doc = parse_file_with_options(input, options)?;

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

    Ok(())
}

fn cmd_json(
    input: &Path,
    output: Option<&Path>,
    compact: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let doc = unpdf::parse_file(input)?;

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

    Ok(())
}

fn cmd_info(input: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Use lenient mode for info command - we want to show metadata even if text extraction fails
    let options = ParseOptions::new().lenient();
    let doc = parse_file_with_options(input, options)?;

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

    Ok(())
}

fn cmd_extract(
    input: &Path,
    output: Option<&Path>,
    pages: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let page_selection = if let Some(p) = pages {
        PageSelection::parse(p).map_err(|e| format!("Invalid page range: {}", e))?
    } else {
        PageSelection::All
    };

    let options = ParseOptions::new().with_pages(page_selection);
    let doc = parse_file_with_options(input, options)?;

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

    Ok(())
}

fn cmd_update(check_only: bool, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Checking for updates...".cyan());

    // Use tokio runtime for async update
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async {
        let status = self_update::backends::github::Update::configure()
            .repo_owner("iyulab")
            .repo_name("unpdf")
            .bin_name("unpdf")
            .show_download_progress(true)
            .current_version(env!("CARGO_PKG_VERSION"))
            .build()?;

        let latest = status.get_latest_release()?;
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
        let latest_ver = semver::Version::parse(latest.version.trim_start_matches('v'))?;

        if latest_ver > current || force {
            if check_only {
                println!(
                    "{} {} -> {}",
                    "Update available:".yellow(),
                    current,
                    latest_ver
                );
            } else {
                println!("{} v{}", "Updating to".green(), latest_ver);
                status.update()?;
                println!("{}", "Update complete!".green().bold());
            }
        } else {
            println!("{} (v{})", "Already up to date".green(), current);
        }

        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

fn cmd_version() {
    println!("{} {}", "unpdf".cyan().bold(), env!("CARGO_PKG_VERSION"));
    println!("PDF content extraction tool");
    println!();
    println!("Repository: {}", "https://github.com/iyulab/unpdf".dimmed());
    println!("License: MIT");
}
