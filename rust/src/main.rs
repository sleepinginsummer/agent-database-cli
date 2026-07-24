mod adapters;
mod config;
mod output;
mod runtime;
mod secrets;
mod security;
mod ssh_tunnel;
mod types;
mod utils;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use types::{MetadataRequest, MetadataType, OutputFormat};

#[derive(Parser)]
#[command(
    name = "agent-database-cli",
    version,
    about = "Unified database command-line tool",
    long_about = "Unified database CLI for MySQL, PostgreSQL, Redis, Oracle, and MongoDB.\n\n\
Connections are defined in ~/.agent-database-cli/config.json (override the path \
with the AGENT_DATABASE_CLI_CONFIG env var); run `list` to see them. Each command \
opens a direct connection, runs, and disconnects.\n\n\
For many queries, stream statements into `repl` (one reused connection), or run the \
`agent-database-cli-mcp` MCP server for a persistent session. Connections are \
read-only by default."
)]
struct Cli {
    #[arg(long, value_parser = ["json", "table", "compact"], help = "Output format (default: the config file's defaultFormat if set, else compact for exec/meta, json otherwise)")]
    format: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "List supported database types and configured connections")]
    List,
    #[command(about = "Test a configured database connection")]
    Test {
        #[arg(long, help = "Configured connection name (see `list`)")]
        db: String,
    },
    #[command(
        name = "exec",
        about = "Execute a SQL query, Redis command, or MongoDB JSON command"
    )]
    Execute {
        #[arg(long, help = "Configured connection name (see `list`)")]
        db: String,
        #[arg(
            long,
            help = "Statement to run: SQL, a Redis command, or a MongoDB JSON command"
        )]
        command: String,
    },
    #[command(
        name = "repl",
        about = "Read SQL from stdin (one statement per line), execute each over a single reused connection, and print one JSON result per line. Lowest per-query latency for many queries."
    )]
    Repl {
        #[arg(long, help = "Configured connection name (see `list`)")]
        db: String,
    },
    #[command(
        name = "meta",
        about = "Query database metadata: tables, columns, collections, or Redis keys"
    )]
    Metadata {
        #[arg(long, help = "Configured connection name (see `list`)")]
        db: String,
        #[arg(
            long = "type",
            value_parser = ["tables", "columns", "collections", "keys"],
            help = "What to inspect: tables (SQL) | columns (SQL, requires --table) | collections (MongoDB) | keys (Redis, optional --pattern)"
        )]
        metadata_type: String,
        #[arg(long, help = "Table name; required when --type is columns")]
        table: Option<String>,
        #[arg(long, help = "Match pattern; used by --type keys (Redis SCAN)")]
        pattern: Option<String>,
    },
    #[command(
        name = "format",
        about = "Show or set the default output format (persisted as defaultFormat in the config file; `--format` still overrides per call)"
    )]
    Format {
        #[arg(
            value_parser = ["json", "table", "compact"],
            help = "Format to persist as the default; omit to show the current setting"
        )]
        format: Option<String>,
        #[arg(
            long,
            conflicts_with = "format",
            help = "Remove the persisted default, restoring the built-in defaults"
        )]
        clear: bool,
    },
    #[command(name = "install-skill", about = "Install or update the Agent skill")]
    InstallSkill(InstallSkillArgs),
}

#[derive(Debug, Args)]
struct InstallSkillArgs {
    #[arg(long, help = "Only show the installation plan, do not write files")]
    dry_run: bool,
    #[arg(long, help = "Skip the interactive confirmation and install directly")]
    yes: bool,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{}", utils::masking::to_error_message(&error));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let format = match &cli.format {
        Some(value) => parse_output_format(value)?,
        None => match config::default_output_format() {
            Some(value) => parse_output_format(&value)
                .context("invalid defaultFormat in the config file")?,
            None => default_format_for(&cli.command),
        },
    };
    let data = match cli.command {
        Commands::List => runtime::run_list().await?,
        Commands::Test { db } => runtime::run_test(&db).await?,
        Commands::Execute { db, command } => {
            serde_json::to_value(runtime::run_execute(&db, &command).await?)?
        }
        Commands::Repl { db } => {
            // repl streams its own JSON-per-line output to stdout.
            runtime::run_repl(&db).await?;
            return Ok(());
        }
        Commands::Metadata {
            db,
            metadata_type,
            table,
            pattern,
        } => {
            let request = MetadataRequest {
                request_type: parse_metadata_type(&metadata_type)?,
                table,
                pattern,
            };
            serde_json::to_value(runtime::run_metadata(&db, request).await?)?
        }
        Commands::Format { format, clear } => {
            if clear {
                let path = config::set_default_output_format(None)?;
                serde_json::json!({ "ok": true, "defaultFormat": null, "configPath": path })
            } else if let Some(value) = format {
                let path = config::set_default_output_format(Some(&value))?;
                serde_json::json!({ "ok": true, "defaultFormat": value, "configPath": path })
            } else {
                serde_json::json!({ "defaultFormat": config::default_output_format() })
            }
        }
        Commands::InstallSkill(args) => install_skill(args)?,
    };
    output::write_output(&data, format)?;
    Ok(())
}

/// Built-in default output format, used when neither `--format` nor the config
/// file's `defaultFormat` is given. Row-producing commands (`exec`, `meta`)
/// default to the token-efficient `compact` form; structural commands keep
/// their natural JSON object so outputs like `{"ok":true}` aren't coerced into
/// a `fields`/`rows` envelope.
fn default_format_for(command: &Commands) -> OutputFormat {
    match command {
        Commands::Execute { .. } | Commands::Metadata { .. } => OutputFormat::Compact,
        _ => OutputFormat::Json,
    }
}

fn parse_output_format(value: &str) -> Result<OutputFormat> {
    match value {
        "json" => Ok(OutputFormat::Json),
        "table" => Ok(OutputFormat::Table),
        "compact" => Ok(OutputFormat::Compact),
        other => anyhow::bail!("unsupported output format: {other}"),
    }
}

fn parse_metadata_type(value: &str) -> Result<MetadataType> {
    match value {
        "tables" => Ok(MetadataType::Tables),
        "columns" => Ok(MetadataType::Columns),
        "collections" => Ok(MetadataType::Collections),
        "keys" => Ok(MetadataType::Keys),
        other => anyhow::bail!("unsupported metadata type: {other}"),
    }
}

#[derive(Debug)]
struct SkillPlanItem {
    path: PathBuf,
    action: String,
    will_write: bool,
    note: String,
}

fn install_skill(args: InstallSkillArgs) -> Result<serde_json::Value> {
    let source = resolve_skill_source()?;
    let home = home_dir()?;
    let main_target = home.join(".agents/skills/agent-database-cli");
    let mut plan = Vec::new();

    plan.push(SkillPlanItem {
        path: main_target.clone(),
        action: if main_target.exists() {
            "update_main"
        } else {
            "create_main"
        }
        .to_string(),
        will_write: true,
        note: format!("copy the built-in skill directory: {}", source.display()),
    });

    for parent in [
        home.join(".codex/skills"),
        home.join(".claude/skills"),
        home.join(".config/agents/skills"),
        home.join(".cursor/skills"),
        home.join(".gemini/skills"),
    ] {
        let target = parent.join("agent-database-cli");
        if !parent.exists() {
            plan.push(SkillPlanItem {
                path: target,
                action: "skip_missing_parent".to_string(),
                will_write: false,
                note: "parent directory does not exist, skipping symlink".to_string(),
            });
            continue;
        }
        match fs::symlink_metadata(&target).ok() {
            Some(meta) if meta.file_type().is_symlink() => plan.push(SkillPlanItem {
                path: target,
                action: "update_symlink".to_string(),
                will_write: true,
                note: format!("update symlink to {}", main_target.display()),
            }),
            Some(_) => plan.push(SkillPlanItem {
                path: target,
                action: "skip_existing_entity".to_string(),
                will_write: false,
                note: "target already exists and is not a symlink, manual handling required; --yes will not overwrite it".to_string(),
            }),
            None => plan.push(SkillPlanItem {
                path: target,
                action: "create_symlink".to_string(),
                will_write: true,
                note: format!("create symlink to {}", main_target.display()),
            }),
        }
    }

    print_skill_plan(&plan, args.dry_run);
    if args.dry_run {
        return Ok(serde_json::json!({ "dry_run": true, "target": main_target }));
    }
    if !args.yes && !confirm_install()? {
        return Ok(serde_json::json!({ "cancelled": true }));
    }

    copy_dir_clean(&source, &main_target)?;
    for item in plan
        .iter()
        .filter(|item| item.action.ends_with("symlink") && item.will_write)
    {
        if let Some(parent) = item.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(&item.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            remove_path(&item.path)?;
        }
        create_symlink_dir(&main_target, &item.path)?;
    }
    Ok(serde_json::json!({
        "installed": true,
        "skill": main_target.join("SKILL.md")
    }))
}

fn print_skill_plan(plan: &[SkillPlanItem], dry_run: bool) {
    println!("agent-database-cli skill installation plan");
    if dry_run {
        println!("mode: dry-run, no files written, no symlinks created");
    }
    for item in plan {
        println!(
            "- [{}] {} -> {} ({})",
            if item.will_write { "write" } else { "skip" },
            item.action,
            item.path.display(),
            item.note
        );
    }
}

fn confirm_install() -> Result<bool> {
    print!("Confirm the installation plan above? Type yes to continue: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim() == "yes")
}

fn resolve_skill_source() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(package_dir) = env::var_os("AGENT_DATABASE_CLI_PACKAGE_DIR") {
        candidates.push(PathBuf::from(package_dir).join("skills/agent-database-cli"));
    }
    if let Ok(exe) = env::current_exe() {
        for ancestor in exe.ancestors().take(6) {
            candidates.push(ancestor.join("skills/agent-database-cli"));
        }
    }
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join("skills/agent-database-cli"));
        candidates.push(cwd.to_path_buf());
    }
    for candidate in candidates {
        if candidate.join("SKILL.md").is_file() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "could not find the built-in skill directory skills/agent-database-cli"
    ))
}

fn copy_dir_clean(source: &Path, target: &Path) -> Result<()> {
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        if fs::symlink_metadata(target)?.file_type().is_symlink() {
            return Err(anyhow!(
                "the main install directory must not be a symlink: {}",
                target.display()
            ));
        }
        fs::remove_dir_all(target).with_context(|| {
            format!(
                "failed to clean the main install directory: {}",
                target.display()
            )
        })?;
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = target.join(entry.file_name());
        let meta = entry.file_type()?;
        if meta.is_dir() {
            copy_dir_clean(&src, &dst)?;
        } else if meta.is_file() {
            fs::copy(&src, &dst)
                .with_context(|| format!("failed to copy file: {}", src.display()))?;
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path)?;
    } else if meta.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target)
        .with_context(|| format!("failed to create symlink: {}", target.display()))
}

#[cfg(windows)]
fn create_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
        .with_context(|| format!("failed to create symlink: {}", target.display()))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("could not locate the user home directory"))
}
