use crate::engine;
use crate::manifest::find_workspace_root;
use crate::models::*;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "monodep", about = "Monorepo dependency deduplication for mise")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install all workspace dependencies + link + dedup
    Install {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, action = clap::ArgAction::Append)]
        filter: Vec<String>,
        #[arg(long)]
        production: bool,
        #[arg(long)]
        no_optional: bool,
    },
    /// Re-link workspace deps + dedup (skip PM install)
    Sync {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, action = clap::ArgAction::Append)]
        filter: Vec<String>,
        #[arg(long)]
        production: bool,
        #[arg(long)]
        no_optional: bool,
    },
    /// Show install plan without executing
    Plan {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, action = clap::ArgAction::Append)]
        filter: Vec<String>,
        #[arg(long)]
        production: bool,
        #[arg(long)]
        no_optional: bool,
    },
    /// Check workspace symlink health
    Doctor {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, action = clap::ArgAction::Append)]
        filter: Vec<String>,
    },
    /// Trace why a dependency exists
    Why {
        dependency: String,
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, action = clap::ArgAction::Append)]
        filter: Vec<String>,
    },
    /// Add a dependency to a workspace
    Add {
        workspace: String,
        package: String,
        #[arg(long, short = 'D')]
        dev: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Update dependencies in a workspace
    Update {
        workspace: String,
        package: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Remove a dependency from a workspace
    Remove {
        workspace: String,
        dependency: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn emit_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

fn make_options(production: bool, no_optional: bool, skip_install: bool) -> SyncOptions {
    SyncOptions {
        include_dev: !production,
        include_optional: !no_optional,
        skip_install,
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Install {
            root,
            filter,
            production,
            no_optional,
        } => {
            let opts = make_options(production, no_optional, false);
            find_workspace_root(&root).and_then(|r| {
                let plan = engine::sync(&r, &filter, &opts)?;
                Ok(serde_json::to_value(&plan).unwrap())
            })
        }
        Commands::Sync {
            root,
            filter,
            production,
            no_optional,
        } => {
            let opts = make_options(production, no_optional, true);
            find_workspace_root(&root).and_then(|r| {
                let plan = engine::sync(&r, &filter, &opts)?;
                Ok(serde_json::to_value(&plan).unwrap())
            })
        }
        Commands::Plan {
            root,
            filter,
            production,
            no_optional,
        } => {
            let opts = make_options(production, no_optional, true);
            find_workspace_root(&root).and_then(|r| {
                let plan = engine::build_plan(&r, &filter, &opts)?;
                Ok(serde_json::to_value(&plan).unwrap())
            })
        }
        Commands::Doctor { root, filter } => {
            let opts = SyncOptions::new();
            find_workspace_root(&root).and_then(|r| {
                let (healthy, payload) = engine::doctor(&r, &filter, &opts)?;
                if !healthy {
                    emit_json(&payload);
                    return Err(MonodepError::General("unhealthy".into()));
                }
                Ok(payload)
            })
        }
        Commands::Why {
            dependency,
            root,
            filter,
        } => {
            let opts = SyncOptions::new();
            find_workspace_root(&root).and_then(|r| engine::why(&r, &dependency, &filter, &opts))
        }
        Commands::Add {
            workspace,
            package,
            dev,
            root,
        } => {
            let opts = SyncOptions::new();
            find_workspace_root(&root)
                .and_then(|r| engine::add_dependency(&r, &workspace, &package, dev, &opts))
        }
        Commands::Update {
            workspace,
            package,
            root,
        } => {
            let opts = SyncOptions::new();
            find_workspace_root(&root)
                .and_then(|r| engine::update_dependency(&r, &workspace, package.as_deref(), &opts))
        }
        Commands::Remove {
            workspace,
            dependency,
            root,
        } => {
            let opts = SyncOptions::new();
            find_workspace_root(&root)
                .and_then(|r| engine::remove_dependency(&r, &workspace, &dependency, &opts))
        }
    };

    match result {
        Ok(value) => {
            emit_json(&value);
            0
        }
        Err(MonodepError::General(msg)) if msg == "unhealthy" => 1,
        Err(e) => {
            eprintln!("monodep: {e}");
            1
        }
    }
}
