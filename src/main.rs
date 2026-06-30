use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use aegis::api::server;
use aegis::{
    AegisConfigStore, AegisSecretStore, AegisStatePaths, BrowserConfig, BrowserMode,
    CredentialInput, NativeConfiguration, app_executable, build_native, configure_native, native,
    replay_trace,
};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "aegis")]
#[command(
    about = "Agentic web browser CLI and runtime control plane",
    long_about = "Aegis is an agentic web browser. Use it to launch the local browser, run one persistent serve process, manage Aegis-owned config and secrets, and control the runtime over a local HTTP API.",
    after_help = CLI_AFTER_HELP
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Path to the native host library. Defaults to the canonical local Release build."
    )]
    #[arg(long, global = true)]
    host_lib: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        default_value = "default",
        help = "Active Aegis profile name under ~/.aegis/profiles/<profile>/..."
    )]
    #[arg(long, global = true, default_value = "default")]
    profile: String,
    #[arg(
        long,
        global = true,
        default_value = "headless",
        help = "Browser mode for serve and runtime operations."
    )]
    #[arg(long, global = true, default_value = "headless")]
    mode: BrowserModeArg,
    #[arg(
        long,
        global = true,
        help = "Initial URL for the runtime. Defaults to the local bootstrap page."
    )]
    #[arg(long, global = true)]
    start_url: Option<String>,
    #[arg(
        long,
        global = true,
        default_value = "127.0.0.1:7878",
        help = "Address of a running `aegis serve` control plane for client commands like search, navigate, and page workflows."
    )]
    #[arg(long, global = true, default_value = "127.0.0.1:7878")]
    server_addr: SocketAddr,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, clap::ValueEnum)]
enum BrowserModeArg {
    Headless,
    Headful,
}

#[derive(Clone, Subcommand)]
enum Commands {
    #[command(about = "Start the headful runtime and open the Linux dashboard")]
    Open,
    #[command(about = "Start the persistent browser runtime and local HTTP control API")]
    Serve {
        #[arg(
            long,
            default_value = "127.0.0.1:7878",
            help = "Address to bind the local HTTP control API."
        )]
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: SocketAddr,
    },
    #[command(about = "Show practical usage guidance for the production CLI workflow")]
    Usage,
    #[command(about = "Show example commands for common Aegis workflows")]
    Examples,
    #[command(about = "Navigate a running Aegis serve runtime to a URL")]
    Navigate { url: String },
    #[command(about = "Run a first-class web search in a running Aegis serve runtime")]
    Search {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        engine: Option<String>,
    },
    #[command(about = "Read and act on the current page through a running Aegis serve runtime")]
    Page {
        #[command(subcommand)]
        command: PageCommands,
    },
    #[command(about = "Replay deterministic traces")]
    Trace {
        #[command(subcommand)]
        command: TraceCommands,
    },
    #[command(about = "Manage Aegis-owned config, secrets, and credentials in ~/.aegis")]
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    #[command(about = "Inspect, build, and install native runtime artifacts")]
    Native {
        #[command(subcommand)]
        command: NativeCommands,
    },
}

#[derive(Clone, Subcommand)]
enum TraceCommands {
    #[command(about = "Inspect a recorded Aegis trace file")]
    Inspect { path: PathBuf },
}

#[derive(Clone, Subcommand)]
enum PageCommands {
    #[command(about = "Inspect the structured current-page research snapshot")]
    Inspect,
    #[command(about = "Print page text from a selected content scope")]
    Text {
        #[arg(
            long,
            help = "Scope to read: full, main, article, controls, or overlays"
        )]
        scope: Option<String>,
    },
    #[command(about = "Print a markdown projection of a selected content scope")]
    Markdown {
        #[arg(
            long,
            help = "Scope to read: full, main, article, controls, or overlays"
        )]
        scope: Option<String>,
    },
    #[command(about = "Summarize the most relevant links, controls, and next actions")]
    Actions,
    #[command(about = "List forms and form controls")]
    Forms,
    #[command(about = "List page headings")]
    Headings,
    #[command(about = "List page links")]
    Links,
    #[command(about = "Find text, headings, or links on the current page")]
    Find {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        exact: bool,
    },
    #[command(about = "Open a page link by text match")]
    OpenLink {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long)]
        exact: bool,
        #[arg(long)]
        href_contains: Option<String>,
        #[arg(long)]
        index: Option<usize>,
    },
}

#[derive(Clone, Subcommand)]
enum ConfigCommands {
    #[command(about = "Read a config concern from ~/.aegis/settings/<concern>.json")]
    Get { concern: String },
    #[command(about = "Write a config concern into ~/.aegis/settings/<concern>.json")]
    Set {
        concern: String,
        #[arg(long)]
        json: String,
    },
    #[command(about = "Read the raw secret payload for a profile")]
    SecretsGet {
        #[arg(long)]
        profile: Option<String>,
    },
    #[command(about = "Write the raw secret payload for a profile")]
    SecretsSet {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: String,
    },
    #[command(about = "List Aegis-owned saved browser credentials for a profile")]
    CredentialsList {
        #[arg(long)]
        profile: Option<String>,
    },
    #[command(about = "Insert or update one saved browser credential for a profile")]
    CredentialsSet {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: String,
    },
    #[command(about = "Remove one saved browser credential by origin and username")]
    CredentialsRemove {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        origin: String,
        #[arg(long)]
        username: String,
    },
    #[command(about = "Clear all saved browser credentials for a profile")]
    CredentialsClear {
        #[arg(long)]
        profile: Option<String>,
    },
}

#[derive(Clone, Subcommand)]
enum NativeCommands {
    #[command(about = "Show resolved native paths and artifact status")]
    Status,
    #[command(about = "Show native preflight readiness, tools, and canonical install paths")]
    Doctor,
    #[command(about = "Generate or refresh native build files")]
    Configure,
    #[command(about = "Build a native target")]
    Build {
        #[arg(long, value_enum, default_value = "release")]
        configuration: NativeConfigurationArg,
        #[arg(long)]
        target: Option<String>,
    },
    #[command(about = "Install the canonical local Release app")]
    Install,
    #[command(about = "Print the canonical native artifact paths")]
    Paths,
}

#[derive(Clone, clap::ValueEnum)]
enum NativeConfigurationArg {
    Debug,
    Release,
}

const CLI_AFTER_HELP: &str = "\
Quick starts:
  aegis
      Start the headful Linux runtime and open the dashboard in your browser.

  aegis open
      Do the same explicitly.

  aegis --mode headful serve --addr 127.0.0.1:7878
      Start the visible browser runtime plus local HTTP API.

  aegis config get credentials
      Inspect credential auto-capture settings.

  aegis examples
      Show more end-to-end commands.";

const USAGE_TEXT: &str = "\
Aegis production usage

1. Install or refresh the canonical local app:
   ./install.sh

2. Human browsing:
   aegis
   aegis open

3. Start the persistent automation runtime:
   aegis --mode headless serve --addr 127.0.0.1:7878
   aegis --mode headful serve --addr 127.0.0.1:7878

4. Manage Aegis-owned state:
   aegis config get agent
   aegis config get credentials
   aegis config credentials-list --profile default

5. Research through a running control plane:
   aegis --server-addr 127.0.0.1:7878 search shopify app review
   aegis --server-addr 127.0.0.1:7878 navigate https://shopify.dev/docs
   aegis --server-addr 127.0.0.1:7878 page text --scope main
   aegis --server-addr 127.0.0.1:7878 page actions
   aegis --server-addr 127.0.0.1:7878 page open-link release an app version

6. Native maintenance:
 aegis native paths
  aegis native doctor
  aegis native build --configuration release --target aegis_host
  aegis native install";

const EXAMPLES_TEXT: &str = "\
Aegis examples

Launch the Linux dashboard:
  aegis
  aegis open

Start a visible runtime for agent debugging:
  aegis --mode headful --profile work serve --addr 127.0.0.1:7878

Start a headless runtime:
  aegis --mode headless serve --addr 127.0.0.1:7878

Inspect local config:
  aegis config get agent
  aegis config get credentials

Disable automatic credential capture:
  aegis config set credentials --json '{\"auto_store\":false}'

List cached credentials for a profile:
  aegis config credentials-list --profile work

Insert a credential manually:
  aegis config credentials-set --profile work --json '{\"origin\":\"https://github.com\",\"username\":\"saint\",\"password\":\"...\",\"username_field\":\"login\",\"password_field\":\"password\",\"form_label\":\"Sign in\"}'

Remove one credential:
  aegis config credentials-remove --profile work --origin https://github.com --username saint

Inspect a trace:
  aegis trace inspect traces/run.fozzy

Research through a running serve process:
  aegis --server-addr 127.0.0.1:7878 search shopify app review
  aegis --server-addr 127.0.0.1:7878 navigate https://shopify.dev/docs
  aegis --server-addr 127.0.0.1:7878 page inspect
  aegis --server-addr 127.0.0.1:7878 page text --scope main
  aegis --server-addr 127.0.0.1:7878 page markdown --scope article
  aegis --server-addr 127.0.0.1:7878 page actions
  aegis --server-addr 127.0.0.1:7878 page forms
  aegis --server-addr 127.0.0.1:7878 page find redirect to your app's ui
  aegis --server-addr 127.0.0.1:7878 page open-link release an app version

Inspect native paths:
  aegis native paths";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let _state_paths = AegisStatePaths::detect()?;
    let current_exe = std::env::current_exe()?;
    let workspace_root = resolve_workspace_root(&current_exe)?;
    let command = resolved_command(&cli);
    let browser_config = BrowserConfig {
        mode: match effective_mode(&cli) {
            BrowserModeArg::Headless => BrowserMode::Headless,
            BrowserModeArg::Headful => BrowserMode::Headful,
        },
        start_url: cli.start_url.clone(),
    };

    match &command {
        Commands::Trace {
            command: TraceCommands::Inspect { path },
        } => {
            let state = replay_trace(path.clone())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "session": state.session,
                    "final_snapshot": state.final_snapshot,
                    "latest_event_sequence": state.events.latest_sequence(),
                    "oldest_retained_event_sequence": state.events.oldest_sequence(),
                    "retained_event_count": state.events.retained_len()
                }))?
            );
            return Ok(());
        }
        Commands::Native { command } => {
            handle_native_command(command.clone(), &workspace_root)?;
            return Ok(());
        }
        Commands::Usage => {
            println!("{USAGE_TEXT}");
            return Ok(());
        }
        Commands::Examples => {
            println!("{EXAMPLES_TEXT}");
            return Ok(());
        }
        Commands::Navigate { url } => {
            let value = serve_json_request(
                cli.server_addr,
                "POST",
                "/navigate",
                Some(&json!({ "url": url })),
            )?;
            print_json_value(&value)?;
            return Ok(());
        }
        Commands::Search { query, engine } => {
            let value = serve_json_request(
                cli.server_addr,
                "POST",
                "/search",
                Some(&json!({
                    "query": query.join(" "),
                    "engine": engine
                })),
            )?;
            print_json_value(&value)?;
            return Ok(());
        }
        Commands::Page { command } => {
            handle_page_command(cli.server_addr, command.clone())?;
            return Ok(());
        }
        Commands::Config { command } => {
            handle_config_command(command.clone(), &cli.profile)?;
            return Ok(());
        }
        _ => {}
    }

    match command {
        Commands::Serve { addr } => {
            let host_lib = cli
                .host_lib
                .clone()
                .unwrap_or_else(|| native::status(&workspace_root).default_host_library);
            if !host_lib.exists() {
                return Err(format!(
                    "host library not found at {}. Run `aegis native build --configuration release --target aegis_host` first or pass --host-lib.",
                    host_lib.display()
                )
                .into());
            }
            server::serve(addr, host_lib, browser_config, cli.profile.clone(), false).await?;
        }
        Commands::Open => {
            let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
            let host_lib = cli
                .host_lib
                .clone()
                .unwrap_or_else(|| native::status(&workspace_root).default_host_library);
            if !host_lib.exists() {
                return Err(format!(
                    "host library not found at {}. Run `aegis native build --configuration release --target aegis_host` first or pass --host-lib.",
                    host_lib.display()
                )
                .into());
            }
            server::serve(
                addr,
                host_lib,
                BrowserConfig {
                    mode: BrowserMode::Headful,
                    start_url: cli.start_url.clone(),
                },
                cli.profile.clone(),
                true,
            )
            .await?;
        }
        Commands::Trace { command } => match command {
            TraceCommands::Inspect { .. } => unreachable!("handled before host init"),
        },
        Commands::Usage => unreachable!("handled before host init"),
        Commands::Examples => unreachable!("handled before host init"),
        Commands::Navigate { .. } => unreachable!("handled before host init"),
        Commands::Search { .. } => unreachable!("handled before host init"),
        Commands::Page { .. } => unreachable!("handled before host init"),
        Commands::Config { .. } => unreachable!("handled before host init"),
        Commands::Native { .. } => unreachable!("handled before runtime startup"),
    }

    Ok(())
}

fn resolve_workspace_root(current_exe: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(root) = std::env::var_os("AEGIS_WORKSPACE_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let cwd = std::env::current_dir()?;
    if is_aegis_workspace_root(&cwd) {
        return Ok(cwd);
    }
    if let Some(root) = find_aegis_workspace_root(current_exe) {
        return Ok(root);
    }
    Ok(cwd)
}

fn is_aegis_workspace_root(path: &Path) -> bool {
    path.join("Cargo.toml").exists() && path.join("native").join("CMakeLists.txt").exists()
}

fn find_aegis_workspace_root(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if is_aegis_workspace_root(ancestor) {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn resolved_command(cli: &Cli) -> Commands {
    if cli.command.is_none() && default_open_shortcut_requested() {
        return Commands::Open;
    }
    resolved_command_for_shortcut(cli, false)
}

fn resolved_command_for_shortcut(cli: &Cli, default_open_shortcut: bool) -> Commands {
    if cli.command.is_none() && default_open_shortcut {
        return Commands::Open;
    }
    cli.command.clone().unwrap_or(Commands::Serve {
        addr: SocketAddr::from(([127, 0, 0, 1], 7878)),
    })
}

fn effective_mode(cli: &Cli) -> BrowserModeArg {
    if matches!(resolved_command(cli), Commands::Open) {
        return BrowserModeArg::Headful;
    }
    cli.mode.clone()
}

fn default_open_shortcut_requested() -> bool {
    std::env::args_os().len() == 1
}

#[derive(Debug, Deserialize)]
struct CliApiErrorBody {
    error: String,
    code: String,
    operation: Option<String>,
    stage: Option<String>,
    elapsed_ms: Option<u64>,
    timed_out: bool,
    restart_recommended: bool,
}

fn print_json_value(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn handle_page_command(
    addr: SocketAddr,
    command: PageCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        PageCommands::Inspect => {
            let value = serve_json_request(addr, "GET", "/page", None)?;
            print_json_value(&value)?;
        }
        PageCommands::Text { scope } => {
            let path = scoped_page_path("/page/text", scope.as_deref());
            let value = serve_json_request(addr, "GET", &path, None)?;
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                println!("{text}");
            } else {
                print_json_value(&value)?;
            }
        }
        PageCommands::Markdown { scope } => {
            let path = scoped_page_path("/page/markdown", scope.as_deref());
            let value = serve_json_request(addr, "GET", &path, None)?;
            if let Some(markdown) = value.get("markdown").and_then(Value::as_str) {
                println!("{markdown}");
            } else {
                print_json_value(&value)?;
            }
        }
        PageCommands::Actions => {
            let value = serve_json_request(addr, "GET", "/page/actions", None)?;
            print_json_value(&value)?;
        }
        PageCommands::Forms => {
            let value = serve_json_request(addr, "GET", "/page/forms", None)?;
            print_json_value(&value)?;
        }
        PageCommands::Headings => {
            let value = serve_json_request(addr, "GET", "/page/headings", None)?;
            print_json_value(&value)?;
        }
        PageCommands::Links => {
            let value = serve_json_request(addr, "GET", "/page/links", None)?;
            print_json_value(&value)?;
        }
        PageCommands::Find { text, exact } => {
            let value = serve_json_request(
                addr,
                "POST",
                "/page/find",
                Some(&json!({
                    "text": text.join(" "),
                    "exact": exact
                })),
            )?;
            print_json_value(&value)?;
        }
        PageCommands::OpenLink {
            text,
            exact,
            href_contains,
            index,
        } => {
            let value = serve_json_request(
                addr,
                "POST",
                "/page/open-link",
                Some(&json!({
                    "text": text.join(" "),
                    "exact": exact,
                    "href_contains": href_contains,
                    "index": index
                })),
            )?;
            print_json_value(&value)?;
        }
    }
    Ok(())
}

fn scoped_page_path(base: &str, scope: Option<&str>) -> String {
    match scope.map(str::trim).filter(|value| !value.is_empty()) {
        Some(scope) => format!("{base}?scope={scope}"),
        None => base.to_string(),
    }
}

fn serve_json_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| {
            format!(
                "could not reach Aegis serve at http://{addr}: {error}. Start it with `aegis serve --addr {addr}` first."
            )
        })?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let body_bytes = if let Some(body) = body {
        serde_json::to_vec(body)?
    } else {
        Vec::new()
    };
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if !body_bytes.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
    }
    request.push_str("\r\n");
    use std::io::{Read, Write};
    stream.write_all(request.as_bytes())?;
    if !body_bytes.is_empty() {
        stream.write_all(&body_bytes)?;
    }
    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes)?;
    let header_end = response_bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("invalid HTTP response from Aegis serve")?;
    let (header_bytes, body_bytes) = response_bytes.split_at(header_end + 4);
    let header_text = std::str::from_utf8(header_bytes)?;
    let status_line = header_text
        .lines()
        .next()
        .ok_or("missing HTTP status line from Aegis serve")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or("missing HTTP status code from Aegis serve")?
        .parse::<u16>()?;
    let body_text = std::str::from_utf8(body_bytes)?.trim();
    if !(200..300).contains(&status) {
        if let Ok(error) = serde_json::from_str::<CliApiErrorBody>(body_text) {
            let mut message = format!(
                "Aegis API {method} {path} failed ({status}): {}",
                error.error
            );
            message.push_str(&format!(" [code={}]", error.code));
            if let Some(operation) = error.operation {
                message.push_str(&format!("\nOperation: {operation}"));
            }
            if let Some(stage) = error.stage {
                message.push_str(&format!("\nStage: {stage}"));
            }
            if let Some(elapsed_ms) = error.elapsed_ms {
                message.push_str(&format!("\nElapsed: {elapsed_ms}ms"));
            }
            if error.timed_out || error.restart_recommended {
                message.push_str(&format!(
                    "\nTimed out: {}\nRestart recommended: {}",
                    error.timed_out, error.restart_recommended
                ));
            }
            return Err(message.into());
        }
        return Err(format!("Aegis API {method} {path} failed ({status}): {body_text}").into());
    }
    if body_text.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(body_text)?)
    }
}

fn handle_native_command(
    command: NativeCommands,
    workspace_root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        NativeCommands::Status => {
            println!(
                "{}",
                serde_json::to_string_pretty(&native::status(workspace_root))?
            );
        }
        NativeCommands::Doctor => {
            println!(
                "{}",
                serde_json::to_string_pretty(&native::doctor(workspace_root))?
            );
        }
        NativeCommands::Configure => {
            let artifact = configure_native(workspace_root)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "configure_artifact": artifact,
                }))?
            );
        }
        NativeCommands::Build {
            configuration,
            target,
        } => {
            let configuration = match configuration {
                NativeConfigurationArg::Debug => NativeConfiguration::Debug,
                NativeConfigurationArg::Release => NativeConfiguration::Release,
            };
            let artifact = build_native(workspace_root, configuration, target.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "configuration": configuration.as_str(),
                    "target": target.unwrap_or_else(|| "aegis_native".to_string()),
                    "artifact": artifact,
                }))?
            );
        }
        NativeCommands::Install => {
            let current_exe = std::env::current_exe()?;
            let app_dir = native::install_local_release(workspace_root, &current_exe)?;
            let status = native::status(workspace_root);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "installed_app_dir": app_dir,
                    "installed_app_executable": app_executable(&app_dir, status.platform),
                    "installed_host_library": status.default_host_library,
                }))?
            );
        }
        NativeCommands::Paths => {
            let status = native::status(workspace_root);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "platform": status.platform,
                    "cef_sdk_root": status.cef_sdk_root,
                    "configure_artifact": status.configure_artifact,
                    "default_app_dir": status.default_app_dir,
                    "default_app_executable": status.default_app_executable,
                    "default_host_library": status.default_host_library,
                }))?
            );
        }
    }

    Ok(())
}

fn handle_config_command(
    command: ConfigCommands,
    default_profile: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ConfigCommands::Get { concern } => {
            let store = AegisConfigStore::detect()?;
            let value = store.get(&concern)?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        ConfigCommands::Set { concern, json } => {
            let store = AegisConfigStore::detect()?;
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let path = store.set(&concern, &value)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "concern": concern,
                    "path": path,
                    "value": value,
                }))?
            );
        }
        ConfigCommands::SecretsGet { profile } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let value = store.load_profile_secrets(&profile)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "secrets": value,
                }))?
            );
        }
        ConfigCommands::SecretsSet { profile, json } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let value: serde_json::Value = serde_json::from_str(&json)?;
            let path = store.save_profile_secrets(&profile, &value)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "path": path,
                    "secrets": value,
                }))?
            );
        }
        ConfigCommands::CredentialsList { profile } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let entries = store.load_profile_credentials(&profile)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "credentials": entries,
                }))?
            );
        }
        ConfigCommands::CredentialsSet { profile, json } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let input: CredentialInput = serde_json::from_str(&json)?;
            let (path, credential) = store.upsert_profile_credential(&profile, input)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "path": path,
                    "credential": credential,
                }))?
            );
        }
        ConfigCommands::CredentialsRemove {
            profile,
            origin,
            username,
        } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let (path, removed) = store.remove_profile_credential(&profile, &origin, &username)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "path": path,
                    "origin": origin,
                    "username": username,
                    "removed": removed,
                }))?
            );
        }
        ConfigCommands::CredentialsClear { profile } => {
            let store = AegisSecretStore::detect()?;
            let profile = profile.unwrap_or_else(|| default_profile.to_string());
            let path = store.clear_profile_credentials(&profile)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "profile": profile,
                    "path": path,
                    "credentials": [],
                }))?
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn no_args_defaults_to_open_shortcut() {
        let cli = parse_cli(&["aegis"]);
        assert!(matches!(
            resolved_command_for_shortcut(&cli, true),
            Commands::Open
        ));
    }

    #[test]
    fn explicit_serve_is_preserved() {
        let cli = parse_cli(&["aegis", "serve"]);
        assert!(matches!(resolved_command(&cli), Commands::Serve { .. }));
    }

    #[test]
    fn runtime_flags_without_subcommand_default_to_serve() {
        let cli = parse_cli(&["aegis", "--mode", "headless"]);
        assert!(matches!(
            resolved_command_for_shortcut(&cli, false),
            Commands::Serve { .. }
        ));
    }

    #[test]
    fn detects_workspace_root_shape() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(is_aegis_workspace_root(root));
    }

    #[test]
    fn finds_workspace_root_from_built_binary_path() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let binary = root.join("target/debug/aegis");
        assert_eq!(find_aegis_workspace_root(&binary).as_deref(), Some(root));
    }
}
