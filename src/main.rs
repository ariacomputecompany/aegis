use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use aegis::api::server;
use aegis::transport::protocol::PROTOCOL_VERSION;
use aegis::{
    AegisConfigStore, AegisSecretStore, BrowserConfig, BrowserMode, CredentialInput,
    NativeConfiguration, app_executable, build_native, canonical_install_host_library,
    configure_native, ensure_workspace_serve_runtime, native, replay_trace, resolve_transfer_paths,
};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "aegis")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(
    about = "Agentic web browser CLI and runtime control plane",
    long_about = "Aegis is an agentic web browser. Use it to launch the local browser, run one persistent serve process, manage Aegis-owned config and secrets, and control the runtime over a local HTTP API.",
    after_help = CLI_AFTER_HELP
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Path to the native host library. By default `serve` uses the workspace release runtime and refreshes it when sources are newer."
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
        help = "Directory where browser-triggered downloads should be saved. Defaults to ~/.aegis/files/downloads."
    )]
    #[arg(long, global = true)]
    download_dir: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Directory where Aegis stages uploaded files before injection. Defaults to ~/.aegis/files/uploads."
    )]
    #[arg(long, global = true)]
    upload_dir: Option<PathBuf>,
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
    #[command(about = "Open the canonical local headful browser app")]
    Open,
    #[command(about = "Print CLI, crate, and protocol version information")]
    Version,
    #[command(about = "Start the persistent browser runtime and local HTTP control API")]
    Serve {
        #[arg(
            long,
            default_value = "127.0.0.1:7878",
            help = "Address to bind the local HTTP control API."
        )]
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: SocketAddr,
        #[arg(
            long,
            help = "Start serve in a detached background process, wait for /version, then print pid/log-path JSON."
        )]
        detach: bool,
        #[arg(
            long,
            requires = "detach",
            help = "Path to the detached serve log file. Defaults to ~/.aegis/logs/serve-<profile>-<addr>.log."
        )]
        log_path: Option<PathBuf>,
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
    #[command(after_help = PAGE_AFTER_HELP)]
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
      Open the local headful browser app from the canonical installed path.

  aegis open
      Open the canonical local app explicitly.

  aegis --mode headful serve --addr 127.0.0.1:7878
      Start the visible browser runtime plus local HTTP API.

  aegis --mode headless serve --detach --addr 127.0.0.1:7878
      Start a background runtime, wait for readiness, and print pid/log-path JSON.

  aegis --mode headless --download-dir /tmp/downloads --upload-dir /tmp/uploads serve
      Override the canonical transfer directories for one runtime.

  aegis --server-addr 127.0.0.1:7878 search shopify app review
      Run a first-class browser search through the persistent runtime.

  aegis --server-addr 127.0.0.1:7878 page actions
      Ask Aegis what the current page is and what the best next actions are.

  aegis config get credentials
      Inspect credential auto-capture settings.

  aegis examples
      Show more end-to-end commands.";

const PAGE_AFTER_HELP: &str = "\
Workflow:
  1. Start `aegis serve`
  2. Run `aegis search ...` or `aegis navigate ...`
  3. Inspect with `aegis page actions` or `aegis page text --scope main`
  4. Continue with `aegis page open-link ...` or `aegis page find ...`

Examples:
  aegis --server-addr 127.0.0.1:7878 page inspect
  aegis --server-addr 127.0.0.1:7878 page text --scope main
  aegis --server-addr 127.0.0.1:7878 page markdown --scope article
  aegis --server-addr 127.0.0.1:7878 page actions
  aegis --server-addr 127.0.0.1:7878 page forms
  aegis --server-addr 127.0.0.1:7878 page open-link release an app version";

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
   aegis --mode headless serve --detach --addr 127.0.0.1:7878

4. Manage Aegis-owned state:
   aegis config get agent
   aegis config get credentials
   aegis config credentials-list --profile default

5. Research through a running control plane:
   aegis --server-addr 127.0.0.1:7878 search shopify app review
   aegis --server-addr 127.0.0.1:7878 navigate https://shopify.dev/docs
   aegis --server-addr 127.0.0.1:7878 page text --scope main
   aegis --server-addr 127.0.0.1:7878 page actions
   aegis --server-addr 127.0.0.1:7878 page find release an app version
   aegis --server-addr 127.0.0.1:7878 page open-link release an app version

6. Native maintenance:
 aegis native paths
  aegis native doctor
  aegis native build --configuration release --target aegis_host
  aegis native install";

const EXAMPLES_TEXT: &str = "\
Aegis examples

Launch the local browser app:
  aegis
  aegis open

Start a visible runtime for agent debugging:
  aegis --mode headful --profile work serve --addr 127.0.0.1:7878

Start a headless runtime:
  aegis --mode headless serve --addr 127.0.0.1:7878

Start a detached headless runtime with an Aegis-managed background launcher:
  aegis --mode headless serve --detach --addr 127.0.0.1:7878
  aegis --mode headless serve --detach --addr 127.0.0.1:7878 --log-path /tmp/aegis.log
  aegis --mode headless --download-dir /tmp/downloads --upload-dir /tmp/uploads serve

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let command = resolved_command(&cli);
    let transfer_paths = resolve_transfer_paths(cli.download_dir.clone(), cli.upload_dir.clone())?;
    let browser_config = BrowserConfig {
        mode: match effective_mode(&cli) {
            BrowserModeArg::Headless => BrowserMode::Headless,
            BrowserModeArg::Headful => BrowserMode::Headful,
        },
        start_url: cli.start_url.clone(),
        download_dir: Some(transfer_paths.download_dir),
        upload_dir: Some(transfer_paths.upload_dir),
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
            let current_exe = std::env::current_exe()?;
            let workspace_root = resolve_workspace_root(&current_exe)?;
            handle_native_command(command.clone(), &workspace_root)?;
            return Ok(());
        }
        Commands::Open => {
            let current_exe = std::env::current_exe()?;
            let workspace_root = resolve_workspace_root(&current_exe)?;
            #[cfg(target_os = "macos")]
            {
                let bundle = native::open_local_app(&workspace_root)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "opened_app_bundle": bundle,
                    }))?
                );
                return Ok(());
            }
            #[cfg(not(target_os = "macos"))]
            {
                let app_dir = native::open_local_app(&workspace_root)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "opened_app_dir": app_dir,
                    }))?
                );
                return Ok(());
            }
        }
        Commands::Usage => {
            println!("{USAGE_TEXT}");
            return Ok(());
        }
        Commands::Version => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "cli_version": env!("CARGO_PKG_VERSION"),
                    "crate_version": env!("CARGO_PKG_VERSION"),
                    "protocol_version": PROTOCOL_VERSION,
                }))?
            );
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
        Commands::Serve {
            addr,
            detach,
            log_path,
        } => {
            let host_lib = if let Some(path) = cli.host_lib.clone() {
                path
            } else {
                let current_exe = std::env::current_exe()?;
                resolve_default_serve_host_lib(&current_exe)?
            };
            if !host_lib.exists() {
                let help = "host library not found at {path}. Run `aegis native build --configuration release --target aegis_host` or pass --host-lib explicitly.";
                return Err(help
                    .replace("{path}", &host_lib.display().to_string())
                    .into());
            }
            if detach {
                let current_exe = std::env::current_exe()?;
                launch_detached_serve(&current_exe, &cli, addr, host_lib, log_path)?;
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            {
                server::serve_main_thread(addr, host_lib, browser_config, cli.profile.clone())?;
            }
            #[cfg(not(target_os = "macos"))]
            {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(server::serve(
                    addr,
                    host_lib,
                    browser_config,
                    cli.profile.clone(),
                ))?;
            }
        }
        Commands::Open => unreachable!("handled before runtime startup"),
        Commands::Trace { command } => match command {
            TraceCommands::Inspect { .. } => unreachable!("handled before host init"),
        },
        Commands::Usage => unreachable!("handled before host init"),
        Commands::Version => unreachable!("handled before host init"),
        Commands::Examples => unreachable!("handled before host init"),
        Commands::Navigate { .. } => unreachable!("handled before host init"),
        Commands::Search { .. } => unreachable!("handled before host init"),
        Commands::Page { .. } => unreachable!("handled before host init"),
        Commands::Config { .. } => unreachable!("handled before host init"),
        Commands::Native { .. } => unreachable!("handled before runtime startup"),
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct CliApiErrorBody {
    error: String,
    code: String,
    suggested_action: Option<String>,
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
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|error| {
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
    let body_slice = &body_bytes[0..body_bytes.len()];
    let body_text = std::str::from_utf8(body_slice)?.trim();
    if !(200..300).contains(&status) {
        if let Ok(error) = serde_json::from_str::<CliApiErrorBody>(body_text) {
            let mut message = format!(
                "Aegis API {method} {path} failed ({status}): {}",
                error.error
            );
            message.push_str(&format!(" [code={}]", error.code));
            if let Some(suggested_action) = error.suggested_action {
                message.push_str(&format!("\nSuggested action: {suggested_action}"));
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

fn resolve_default_serve_host_lib(
    current_exe: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match default_serve_runtime_source(current_exe) {
        ServeRuntimeSource::Workspace(root) => Ok(ensure_workspace_serve_runtime(root)?),
        ServeRuntimeSource::Installed(installed_host_lib) => {
            if installed_host_lib.exists() {
                Ok(installed_host_lib)
            } else {
                Err(format!(
                    "installed Aegis runtime not found at {}. Reinstall Aegis with ./install.sh or pass --host-lib explicitly.",
                    installed_host_lib.display()
                )
                .into())
            }
        }
        ServeRuntimeSource::MissingInstalled => Err(
            "unable to resolve a default Aegis host runtime. Reinstall Aegis or pass --host-lib explicitly."
                .into(),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ServeRuntimeSource {
    Workspace(PathBuf),
    Installed(PathBuf),
    MissingInstalled,
}

fn default_serve_runtime_source(current_exe: &Path) -> ServeRuntimeSource {
    if let Some(root) = std::env::var_os("AEGIS_WORKSPACE_ROOT") {
        return ServeRuntimeSource::Workspace(PathBuf::from(root));
    }
    if let Some(root) = find_aegis_workspace_root(current_exe) {
        return ServeRuntimeSource::Workspace(root);
    }
    if let Some(installed_host_lib) = canonical_install_host_library() {
        return ServeRuntimeSource::Installed(installed_host_lib);
    }
    ServeRuntimeSource::MissingInstalled
}

fn launch_detached_serve(
    current_exe: &Path,
    cli: &Cli,
    addr: SocketAddr,
    host_lib: PathBuf,
    log_path_override: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let log_path = detach_log_path(&cli.profile, addr, log_path_override);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let stderr = stdout.try_clone()?;

    let mut child = ProcessCommand::new(current_exe);
    child.args(detached_serve_child_args(cli, addr, &host_lib));
    child.stdin(Stdio::null());
    child.stdout(Stdio::from(stdout));
    child.stderr(Stdio::from(stderr));
    #[cfg(unix)]
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = child.spawn()?;
    wait_for_detached_serve(addr, &mut child, &log_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "mode": "detached_serve",
            "pid": child.id(),
            "addr": addr,
            "profile": cli.profile,
            "log_path": log_path,
            "runtime_executable": current_exe,
        }))?
    );
    Ok(())
}

fn detached_serve_child_args(cli: &Cli, addr: SocketAddr, host_lib: &Path) -> Vec<String> {
    let mut args = vec![
        "--mode".to_string(),
        effective_mode(cli).as_cli_value().to_string(),
        "--profile".to_string(),
        cli.profile.clone(),
        "--host-lib".to_string(),
        host_lib.display().to_string(),
    ];
    if let Some(start_url) = cli.start_url.as_ref() {
        args.push("--start-url".to_string());
        args.push(start_url.clone());
    }
    if let Some(download_dir) = cli.download_dir.as_ref() {
        args.push("--download-dir".to_string());
        args.push(download_dir.display().to_string());
    }
    if let Some(upload_dir) = cli.upload_dir.as_ref() {
        args.push("--upload-dir".to_string());
        args.push(upload_dir.display().to_string());
    }
    args.push("serve".to_string());
    args.push("--addr".to_string());
    args.push(addr.to_string());
    args
}

fn detach_log_path(profile: &str, addr: SocketAddr, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(path) = override_path {
        return path;
    }
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".aegis").join("logs"))
        .unwrap_or_else(std::env::temp_dir);
    let addr_slug = addr.to_string().replace([':', '.'], "_");
    base.join(format!("serve-{profile}-{addr_slug}.log"))
}

fn wait_for_detached_serve(
    addr: SocketAddr,
    child: &mut Child,
    log_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            let tail = read_log_tail(log_path);
            return Err(format!(
                "detached Aegis serve exited before it became reachable (status: {status}). Log: {}{}",
                log_path.display(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("\n{tail}")
                }
            )
            .into());
        }
        if detached_control_plane_ready(addr) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let tail = read_log_tail(log_path);
            return Err(format!(
                "detached Aegis serve did not become reachable at http://{addr} within 15s. Log: {}{}",
                log_path.display(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("\n{tail}")
                }
            )
            .into());
        }
        sleep(Duration::from_millis(100));
    }
}

fn detached_control_plane_ready(addr: SocketAddr) -> bool {
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let request = format!("GET /version HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buffer = [0_u8; 128];
    match stream.read(&mut buffer) {
        Ok(read) if read > 0 => std::str::from_utf8(&buffer[..read])
            .ok()
            .is_some_and(|text| text.contains("HTTP/1.1")),
        _ => false,
    }
}

fn read_log_tail(log_path: &Path) -> String {
    let Ok(contents) = fs::read_to_string(log_path) else {
        return String::new();
    };
    let lines = contents.lines().collect::<Vec<_>>();
    let tail = lines.iter().rev().take(12).copied().collect::<Vec<_>>();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
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
        detach: false,
        log_path: None,
    })
}

fn effective_mode(cli: &Cli) -> BrowserModeArg {
    if matches!(resolved_command(cli), Commands::Open) {
        return BrowserModeArg::Headful;
    }
    cli.mode.clone()
}

impl BrowserModeArg {
    fn as_cli_value(&self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Headful => "headful",
        }
    }
}

fn default_open_shortcut_requested() -> bool {
    std::env::args_os().len() == 1
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
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    fn home_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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
    fn detached_serve_flag_is_parsed() {
        let cli = parse_cli(&[
            "aegis",
            "--mode",
            "headless",
            "--profile",
            "work",
            "serve",
            "--detach",
            "--addr",
            "127.0.0.1:7900",
            "--log-path",
            "/tmp/aegis.log",
        ]);
        let Commands::Serve {
            addr,
            detach,
            log_path,
        } = resolved_command(&cli)
        else {
            panic!("serve command should be resolved");
        };
        assert!(detach);
        assert_eq!(addr, SocketAddr::from(([127, 0, 0, 1], 7900)));
        assert_eq!(log_path, Some(PathBuf::from("/tmp/aegis.log")));
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

    #[test]
    fn serve_prefers_workspace_runtime_source_for_workspace_binary() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let binary = root.join("target/debug/aegis");
        assert_eq!(
            default_serve_runtime_source(&binary),
            ServeRuntimeSource::Workspace(root.to_path_buf())
        );
    }

    #[test]
    fn serve_prefers_installed_runtime_source_for_non_workspace_binary() {
        let _guard = home_env_lock()
            .lock()
            .expect("home env lock should not poison");
        let temp = tempfile::tempdir().expect("temporary dir should be created");
        let home_dir = temp.path().join("home");
        let installed_host = home_dir
            .join("Applications")
            .join("Aegis.app")
            .join("Contents")
            .join("Frameworks")
            .join("libaegis_host.dylib");
        fs::create_dir_all(
            installed_host
                .parent()
                .expect("installed host should have a parent"),
        )
        .expect("installed host dir should be created");
        fs::write(&installed_host, b"host").expect("installed host should be created");
        unsafe {
            std::env::set_var("HOME", &home_dir);
            std::env::remove_var("AEGIS_WORKSPACE_ROOT");
        }
        let non_workspace_binary = temp.path().join("bin").join("aegis");
        assert_eq!(
            default_serve_runtime_source(&non_workspace_binary),
            ServeRuntimeSource::Installed(installed_host)
        );
    }

    #[test]
    fn detached_serve_child_args_preserve_runtime_flags() {
        let cli = parse_cli(&[
            "aegis",
            "--mode",
            "headful",
            "--profile",
            "work",
            "--start-url",
            "http://127.0.0.1:3000",
            "--download-dir",
            "/tmp/downloads",
            "--upload-dir",
            "/tmp/uploads",
            "serve",
            "--detach",
            "--addr",
            "127.0.0.1:7900",
        ]);
        let args = detached_serve_child_args(
            &cli,
            SocketAddr::from(([127, 0, 0, 1], 7900)),
            Path::new("/tmp/libaegis_host.dylib"),
        );
        assert_eq!(
            args,
            vec![
                "--mode".to_string(),
                "headful".to_string(),
                "--profile".to_string(),
                "work".to_string(),
                "--host-lib".to_string(),
                "/tmp/libaegis_host.dylib".to_string(),
                "--start-url".to_string(),
                "http://127.0.0.1:3000".to_string(),
                "--download-dir".to_string(),
                "/tmp/downloads".to_string(),
                "--upload-dir".to_string(),
                "/tmp/uploads".to_string(),
                "serve".to_string(),
                "--addr".to_string(),
                "127.0.0.1:7900".to_string(),
            ]
        );
    }
}
