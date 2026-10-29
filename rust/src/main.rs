mod adapters;
mod config;
mod daemon;
mod editor;
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
#[command(name = "agent-database-cli", version, about = "统一数据库命令行工具")]
struct Cli {
    #[arg(long, default_value = "json", value_parser = ["json", "table"])]
    format: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "展示支持的数据库类型和本地连接，可按分组或标签筛选")]
    List {
        #[arg(long, value_parser = parse_classification_filter, help = "按分组精确筛选")]
        group: Option<String>,
        #[arg(long, value_parser = parse_classification_filter, help = "按标签精确筛选，与分组取交集")]
        tag: Option<String>,
    },
    #[command(about = "测试数据库连接")]
    Test {
        #[arg(long)]
        db: String,
    },
    #[command(name = "exec", about = "统一执行 SQL、Redis 命令或 MongoDB JSON 命令")]
    Execute {
        #[arg(long)]
        db: String,
        #[arg(long)]
        command: String,
    },
    #[command(name = "meta", about = "查询数据库元信息")]
    Metadata {
        #[arg(long)]
        db: String,
        #[arg(long = "type", value_parser = ["tables", "columns", "collections", "keys"])]
        metadata_type: String,
        #[arg(long)]
        table: Option<String>,
        #[arg(long)]
        pattern: Option<String>,
    },
    #[command(about = "重置指定数据库连接")]
    Reset {
        #[arg(long)]
        db: String,
    },
    #[command(about = "管理本地连接守护进程")]
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
    #[command(name = "editor", about = "可视化编辑本地配置文件")]
    Editor {
        #[command(subcommand)]
        command: EditorCommands,
    },
    #[command(name = "install-skill", about = "安装或更新 Agent skill")]
    InstallSkill(InstallSkillArgs),
}

#[derive(Debug, Args)]
struct InstallSkillArgs {
    #[arg(long, help = "只展示安装计划，不写入文件")]
    dry_run: bool,
    #[arg(long, help = "跳过交互确认，直接执行安装")]
    yes: bool,
}

#[derive(Subcommand)]
enum DaemonCommands {
    #[command(about = "启动 daemon")]
    Start,
    #[command(about = "停止 daemon")]
    Stop,
    #[command(about = "查看 daemon 状态")]
    Status,
    #[command(hide = true)]
    Run,
}

#[derive(Subcommand)]
enum EditorCommands {
    #[command(about = "启动编辑服务并打开页面")]
    Start {
        #[arg(long, help = "监听端口，默认 0 表示随机空闲端口")]
        port: Option<u16>,
        #[arg(long, help = "启动后不自动打开浏览器")]
        no_open: bool,
    },
    #[command(about = "关闭编辑服务")]
    Stop,
    #[command(about = "查看编辑服务状态")]
    Status,
    #[command(hide = true)]
    Run {
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        config_path: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    // rustls 的 ring 与 aws-lc-rs 同时启用时没有进程级默认 provider，首个 TLS 建连会 panic。
    let _ = rustls::crypto::ring::default_provider().install_default();
    if let Err(error) = run().await {
        eprintln!("{}", utils::masking::to_error_message(&error));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let format = parse_output_format(&cli.format)?;
    let data = match cli.command {
        Commands::List { group, tag } => {
            runtime::run_list(group.as_deref(), tag.as_deref()).await?
        }
        Commands::Test { db } => runtime::run_test(&db).await?,
        Commands::Execute { db, command } => {
            serde_json::to_value(runtime::run_execute(&db, &command).await?)?
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
        Commands::Reset { db } => runtime::run_reset(&db).await?,
        Commands::Daemon { command } => match command {
            DaemonCommands::Start => daemon::control::start_daemon().await?,
            DaemonCommands::Stop => daemon::control::stop_daemon().await?,
            DaemonCommands::Status => daemon::control::daemon_status().await?,
            DaemonCommands::Run => {
                daemon::server::run_server().await?;
                serde_json::json!({ "stopped": true })
            }
        },
        Commands::Editor { command } => match command {
            EditorCommands::Start { port, no_open } => {
                editor::control::start_editor(port.unwrap_or(0), !no_open).await?
            }
            EditorCommands::Stop => editor::control::stop_editor().await?,
            EditorCommands::Status => editor::control::editor_status().await?,
            EditorCommands::Run {
                port,
                token,
                config_path,
            } => {
                editor::server::run_editor_server(port, token, config_path.map(PathBuf::from))
                    .await?;
                serde_json::json!({ "stopped": true })
            }
        },
        Commands::InstallSkill(args) => install_skill(args)?,
    };
    output::write_output(&data, format)?;
    Ok(())
}

#[cfg(test)]
mod classification_tests {
    use super::*;

    #[test]
    fn list_parses_group_and_tag_in_either_order() {
        for args in [
            vec![
                "agent-database-cli",
                "list",
                "--group",
                " 生产 ",
                "--tag",
                "应用",
            ],
            vec![
                "agent-database-cli",
                "list",
                "--tag",
                "应用",
                "--group",
                "生产",
            ],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            match cli.command {
                Commands::List { group, tag } => {
                    assert_eq!(group.as_deref(), Some("生产"));
                    assert_eq!(tag.as_deref(), Some("应用"));
                }
                _ => panic!("应解析为 list"),
            }
        }
        for args in [
            vec!["agent-database-cli", "list", "--tag"],
            vec!["agent-database-cli", "list", "--group", " "],
            vec!["agent-database-cli", "list", "--tag", "a", "--tag", "b"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}

fn parse_classification_filter(value: &str) -> std::result::Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("分组或标签不能为空".to_string());
    }
    Ok(value.to_string())
}

fn parse_output_format(value: &str) -> Result<OutputFormat> {
    match value {
        "json" => Ok(OutputFormat::Json),
        "table" => Ok(OutputFormat::Table),
        other => anyhow::bail!("不支持的输出格式: {other}"),
    }
}

fn parse_metadata_type(value: &str) -> Result<MetadataType> {
    match value {
        "tables" => Ok(MetadataType::Tables),
        "columns" => Ok(MetadataType::Columns),
        "collections" => Ok(MetadataType::Collections),
        "keys" => Ok(MetadataType::Keys),
        other => anyhow::bail!("不支持的元信息类型: {other}"),
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
        note: format!("复制内置 skill 目录: {}", source.display()),
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
                note: "父目录不存在，跳过软链接".to_string(),
            });
            continue;
        }
        match fs::symlink_metadata(&target).ok() {
            Some(meta) if meta.file_type().is_symlink() => plan.push(SkillPlanItem {
                path: target,
                action: "update_symlink".to_string(),
                will_write: true,
                note: format!("更新软链接到 {}", main_target.display()),
            }),
            Some(_) => plan.push(SkillPlanItem {
                path: target,
                action: "skip_existing_entity".to_string(),
                will_write: false,
                note: "目标已存在且不是软链接，必须手动处理，--yes 也不会覆盖".to_string(),
            }),
            None => plan.push(SkillPlanItem {
                path: target,
                action: "create_symlink".to_string(),
                will_write: true,
                note: format!("创建软链接到 {}", main_target.display()),
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
    println!("agent-database-cli skill 安装计划");
    if dry_run {
        println!("模式: dry-run，不写文件、不创建软链接");
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
    print!("确认执行以上安装计划？输入 yes 继续: ");
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
    Err(anyhow!("找不到内置 skill 目录 skills/agent-database-cli"))
}

fn copy_dir_clean(source: &Path, target: &Path) -> Result<()> {
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        if fs::symlink_metadata(target)?.file_type().is_symlink() {
            return Err(anyhow!("主安装目录不能是软链接: {}", target.display()));
        }
        fs::remove_dir_all(target)
            .with_context(|| format!("清理主安装目录失败: {}", target.display()))?;
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
            fs::copy(&src, &dst).with_context(|| format!("复制文件失败: {}", src.display()))?;
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
        .with_context(|| format!("创建软链接失败: {}", target.display()))
}

#[cfg(windows)]
fn create_symlink_dir(source: &Path, target: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
        .with_context(|| format!("创建软链接失败: {}", target.display()))
}

fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("无法定位用户主目录"))
}
