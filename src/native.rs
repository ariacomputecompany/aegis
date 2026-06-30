use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::transport::bridge::AegisError;

const NATIVE_DIR: &str = "native";
const DEFAULT_TARGET: &str = "aegis_native";
const HOST_LIBRARY_TARGET: &str = "aegis_host";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePlatform {
    Macos,
    Linux,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeStatus {
    pub platform: NativePlatform,
    pub cef_sdk_root: PathBuf,
    pub cef_sdk_present: bool,
    pub build_dir: PathBuf,
    pub configure_artifact: PathBuf,
    pub configure_artifact_present: bool,
    pub default_app_dir: PathBuf,
    pub default_app_dir_present: bool,
    pub default_app_executable: PathBuf,
    pub default_app_executable_present: bool,
    pub default_host_library: PathBuf,
    pub default_host_library_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeToolStatus {
    pub name: String,
    pub found: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeDoctor {
    pub status: NativeStatus,
    pub canonical_install_dir: Option<PathBuf>,
    pub canonical_install_app_executable: Option<PathBuf>,
    pub canonical_install_cli: Option<PathBuf>,
    pub canonical_install_host_library: Option<PathBuf>,
    pub workspace_app_dir: PathBuf,
    pub workspace_app_executable: PathBuf,
    pub workspace_app_executable_present: bool,
    pub workspace_host_library: PathBuf,
    pub workspace_host_library_present: bool,
    pub required_tools: Vec<NativeToolStatus>,
    pub missing_tools: Vec<String>,
    pub ready_for_configure: bool,
    pub ready_for_build: bool,
    pub ready_for_install: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum NativeConfiguration {
    Debug,
    Release,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct CodeSigningConfig {
    identity: String,
    options: Option<String>,
    entitlements: Option<PathBuf>,
}

impl NativeConfiguration {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }
}

pub fn current_platform() -> NativePlatform {
    #[cfg(target_os = "macos")]
    {
        NativePlatform::Macos
    }
    #[cfg(target_os = "linux")]
    {
        NativePlatform::Linux
    }
}

pub fn status(root: impl AsRef<Path>) -> NativeStatus {
    let root = root.as_ref();
    let platform = current_platform();
    let cef_sdk_root = root.join(cef_sdk_dir(platform));
    let build_dir = build_dir(root, platform);
    let configure_artifact = configure_artifact(root, platform);
    let default_app_dir = preferred_app_dir(root, platform);
    let default_app_executable = app_executable(&default_app_dir, platform);
    let default_host_library = preferred_host_library(root, platform, &default_app_dir);

    NativeStatus {
        platform,
        cef_sdk_root: cef_sdk_root.clone(),
        cef_sdk_present: cef_sdk_root.exists(),
        build_dir: build_dir.clone(),
        configure_artifact_present: configure_artifact.exists(),
        configure_artifact,
        default_app_dir_present: default_app_dir.exists(),
        default_app_dir,
        default_app_executable_present: default_app_executable.exists(),
        default_app_executable,
        default_host_library_present: default_host_library.exists(),
        default_host_library,
    }
}

pub fn doctor(root: impl AsRef<Path>) -> NativeDoctor {
    let root = root.as_ref();
    let status = status(root);
    let platform = status.platform;
    let canonical_install_dir = canonical_install_dir(platform);
    let canonical_install_app_executable = canonical_install_dir
        .as_ref()
        .map(|app_dir| canonical_app_executable_path(app_dir, platform));
    let canonical_install_cli = canonical_install_dir
        .as_ref()
        .map(|app_dir| bundled_cli_path(app_dir, platform));
    let canonical_install_host_library = canonical_install_dir
        .as_ref()
        .map(|app_dir| bundled_host_library(app_dir, platform));
    let workspace_app_dir = default_workspace_app_dir(root, platform);
    let workspace_app_executable = app_executable(&workspace_app_dir, platform);
    let workspace_host_library = workspace_host_library(root, platform);
    let required_tools = required_command_names(platform)
        .into_iter()
        .map(|name| NativeToolStatus {
            name: name.to_string(),
            found: command_exists(name),
        })
        .collect::<Vec<_>>();
    let missing_tools = required_tools
        .iter()
        .filter(|tool| !tool.found)
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let mut notes = Vec::new();

    if !status.cef_sdk_present {
        notes.push(format!(
            "Install the platform CEF bundle under {} before configuring or building native artifacts.",
            status.cef_sdk_root.display()
        ));
    }
    if !status.configure_artifact_present {
        notes.push("Run `aegis native configure` to generate native build files.".into());
    }
    if !workspace_host_library.exists() {
        notes.push(format!(
            "Build the native host library so {} exists.",
            workspace_host_library.display()
        ));
    }
    if !workspace_app_executable.exists() {
        notes.push(format!(
            "Build the native app target so {} exists.",
            workspace_app_executable.display()
        ));
    }
    if canonical_install_cli
        .as_ref()
        .is_some_and(|path| !path.exists())
    {
        notes.push(
            "Run `aegis native install` or `./install.sh` to refresh the canonical local app."
                .into(),
        );
    }

    let ready_for_configure = missing_tools.is_empty() && status.cef_sdk_present;
    let ready_for_build = ready_for_configure && status.configure_artifact_present;
    let ready_for_install =
        ready_for_configure && workspace_host_library.exists() && workspace_app_executable.exists();

    NativeDoctor {
        status,
        canonical_install_dir,
        canonical_install_app_executable,
        canonical_install_cli,
        canonical_install_host_library,
        workspace_app_dir,
        workspace_app_executable_present: workspace_app_executable.exists(),
        workspace_app_executable,
        workspace_host_library_present: workspace_host_library.exists(),
        workspace_host_library,
        required_tools,
        missing_tools,
        ready_for_configure,
        ready_for_build,
        ready_for_install,
        notes,
    }
}

pub fn configure_native(root: impl AsRef<Path>) -> Result<PathBuf, AegisError> {
    configure_native_for(root, NativeConfiguration::Release)
}

fn configure_native_for(
    root: impl AsRef<Path>,
    configuration: NativeConfiguration,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let platform = current_platform();
    let native_dir = root.join(NATIVE_DIR);
    let build_dir = build_dir(root, platform);
    let configure_artifact = configure_artifact(root, platform);

    if build_dir.exists()
        && (!native_build_tree_healthy(&build_dir, &configure_artifact, platform)
            || native_build_tree_stale(&native_dir, &configure_artifact)?)
    {
        fs::remove_dir_all(&build_dir)?;
    }
    fs::create_dir_all(&build_dir)?;

    let args = configure_args(root, &native_dir, &build_dir, platform, configuration)?;
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    if let Err(error) = run_checked("cmake", &borrowed, root) {
        if native_build_tree_can_retry(&build_dir, &configure_artifact) {
            fs::remove_dir_all(&build_dir)?;
            fs::create_dir_all(&build_dir)?;
            let retry_args =
                configure_args(root, &native_dir, &build_dir, platform, configuration)?;
            let retry_borrowed = retry_args.iter().map(String::as_str).collect::<Vec<_>>();
            run_checked("cmake", &retry_borrowed, root)?;
        } else {
            return Err(error);
        }
    }
    Ok(configure_artifact)
}

pub fn build_native(
    root: impl AsRef<Path>,
    configuration: NativeConfiguration,
    target: Option<&str>,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let platform = current_platform();
    let build_dir = build_dir(root, platform);
    configure_native_for(root, configuration)?;

    let target = target.unwrap_or(DEFAULT_TARGET);
    let mut args = vec![
        "--build".to_string(),
        path_str(&build_dir)?.to_string(),
        "--target".to_string(),
        target.to_string(),
    ];
    if platform == NativePlatform::Macos {
        args.push("--config".to_string());
        args.push(configuration.as_str().to_string());
    } else if let Ok(parallelism) = std::thread::available_parallelism() {
        args.push("--parallel".to_string());
        args.push(parallelism.get().to_string());
    }
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_checked("cmake", &borrowed, root)?;
    Ok(artifact_for_target(root, configuration, target))
}

pub fn ensure_workspace_serve_runtime(root: impl AsRef<Path>) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let platform = current_platform();
    let workspace_app = default_workspace_app_dir(root, platform);
    let workspace_app_executable = app_executable(&workspace_app, platform);
    let workspace_host = workspace_host_library(root, platform);

    if workspace_runtime_is_current(root, &workspace_app_executable, &workspace_host)? {
        return Ok(workspace_host);
    }

    build_native(
        root,
        NativeConfiguration::Release,
        Some(HOST_LIBRARY_TARGET),
    )?;
    build_native(root, NativeConfiguration::Release, Some(DEFAULT_TARGET))?;

    Ok(workspace_host_library(root, platform))
}

pub fn artifact_for_target(
    root: impl AsRef<Path>,
    configuration: NativeConfiguration,
    target: &str,
) -> PathBuf {
    let root = root.as_ref();
    let platform = current_platform();
    let output_dir = artifact_output_dir(root, platform, configuration);
    match (platform, target) {
        (NativePlatform::Macos, HOST_LIBRARY_TARGET) => output_dir.join("libaegis_host.dylib"),
        (NativePlatform::Linux, HOST_LIBRARY_TARGET) => output_dir.join("libaegis_host.so"),
        (NativePlatform::Macos, _) => output_dir.join("aegis_native.app"),
        (NativePlatform::Linux, "aegis_helper") => output_dir.join("aegis_helper"),
        (NativePlatform::Linux, _) => output_dir.join("aegis_native"),
    }
}

pub fn app_executable(app_dir: impl AsRef<Path>, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => {
            let app_dir = app_dir.as_ref();
            let binary_name = macos_bundle_executable_name(app_dir)
                .or_else(|| detect_macos_bundle_executable(app_dir))
                .unwrap_or_else(|| {
                    app_dir
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| DEFAULT_TARGET.to_string())
                });
            app_dir.join("Contents").join("MacOS").join(binary_name)
        }
        NativePlatform::Linux => {
            let app_dir = app_dir.as_ref();
            let bundled = app_dir.join("bin").join(DEFAULT_TARGET);
            if bundled.exists() || app_dir.join("bin").exists() {
                bundled
            } else {
                app_dir.join(DEFAULT_TARGET)
            }
        }
    }
}

pub fn install_local_release(
    root: impl AsRef<Path>,
    source_executable: impl AsRef<Path>,
) -> Result<PathBuf, AegisError> {
    let root = root.as_ref();
    let source_executable = source_executable.as_ref();
    let platform = current_platform();
    let install_dir = installed_app_dir(platform).ok_or_else(|| {
        AegisError::Bridge("HOME is not set; cannot resolve local app install path".into())
    })?;
    let build_output_dir = artifact_for_target(root, NativeConfiguration::Release, DEFAULT_TARGET);
    let built_host_library =
        artifact_for_target(root, NativeConfiguration::Release, HOST_LIBRARY_TARGET);

    build_native(
        root,
        NativeConfiguration::Release,
        Some(HOST_LIBRARY_TARGET),
    )?;
    build_native(root, NativeConfiguration::Release, None)?;

    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)?;
    }
    if let Some(parent) = install_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    match platform {
        NativePlatform::Macos => {
            copy_dir_recursive(&build_output_dir, &install_dir)?;
            let bundled_cli = install_dir.join("Contents").join("MacOS").join("aegis_cli");
            copy_file_with_mode(source_executable, &bundled_cli)?;
            let bundled_host_library = bundled_host_library(&install_dir, platform);
            if let Some(parent) = bundled_host_library.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_file_with_mode(&built_host_library, &bundled_host_library)?;
            clear_quarantine_attribute(&install_dir);
            sign_bundle_for_distribution(&install_dir)?;
            verify_signed_bundle(&install_dir)?;
        }
        NativePlatform::Linux => {
            fs::create_dir_all(install_dir.join("bin"))?;
            fs::create_dir_all(install_dir.join("lib"))?;
            copy_file_with_mode(
                &build_output_dir,
                &install_dir.join("bin").join(DEFAULT_TARGET),
            )?;
            copy_file_with_mode(
                source_executable,
                &install_dir.join("bin").join("aegis_cli"),
            )?;
            copy_file_with_mode(
                &built_host_library,
                &install_dir.join("lib").join("libaegis_host.so"),
            )?;
            let workspace_lib_dir = build_output_dir
                .parent()
                .ok_or_else(|| AegisError::Bridge("linux build output missing parent".into()))?;
            copy_linux_runtime_artifacts(workspace_lib_dir, &install_dir.join("lib"))?;
        }
    }

    Ok(install_dir)
}

pub fn open_local_app(root: impl AsRef<Path>) -> Result<PathBuf, AegisError> {
    let platform = current_platform();
    let app_dir = preferred_app_dir(root.as_ref(), platform);
    let executable = app_executable(&app_dir, platform);
    if !executable.exists() {
        return Err(AegisError::Bridge(format!(
            "app executable not found at {}. Run `./install.sh` to install the canonical local release first.",
            executable.display()
        )));
    }

    match platform {
        NativePlatform::Macos => {
            run_checked("open", &[path_str(&app_dir)?], Path::new("/"))?;
        }
        NativePlatform::Linux => {
            Command::new(&executable).spawn()?;
        }
    }

    Ok(app_dir)
}

impl NativePlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
}

fn configure_generator(platform: NativePlatform) -> Option<&'static str> {
    match platform {
        NativePlatform::Macos => Some("Xcode"),
        NativePlatform::Linux => None,
    }
}

fn configure_args(
    root: &Path,
    native_dir: &Path,
    build_dir: &Path,
    platform: NativePlatform,
    configuration: NativeConfiguration,
) -> Result<Vec<String>, AegisError> {
    let mut args = vec![
        "-S".to_string(),
        path_str(native_dir)?.to_string(),
        "-B".to_string(),
        path_str(build_dir)?.to_string(),
        format!("-DAEGIS_TARGET_PLATFORM={}", platform.as_str()),
        format!(
            "-DCEF_ROOT={}",
            path_str(&root.join(cef_sdk_dir(platform)))?
        ),
    ];
    if let Some(generator) = configure_generator(platform) {
        args.push("-G".to_string());
        args.push(generator.to_string());
    }
    if platform == NativePlatform::Macos {
        args.push(format!("-DPROJECT_ARCH={}", apple_arch()));
    } else {
        args.push(format!("-DCMAKE_BUILD_TYPE={}", configuration.as_str()));
    }
    Ok(args)
}

fn native_build_tree_healthy(
    build_dir: &Path,
    configure_artifact: &Path,
    platform: NativePlatform,
) -> bool {
    if configure_artifact.exists() {
        return true;
    }

    let cache_path = build_dir.join("CMakeCache.txt");
    if !cache_path.exists() {
        return false;
    }

    let Ok(cache) = fs::read_to_string(cache_path) else {
        return false;
    };
    match platform {
        NativePlatform::Macos => cache.contains("CMAKE_GENERATOR:INTERNAL=Xcode"),
        NativePlatform::Linux => cache.contains("CMAKE_GENERATOR:INTERNAL=Unix Makefiles"),
    }
}

fn native_build_tree_can_retry(build_dir: &Path, configure_artifact: &Path) -> bool {
    build_dir.exists() && !configure_artifact.exists()
}

fn native_build_tree_stale(
    native_dir: &Path,
    configure_artifact: &Path,
) -> Result<bool, AegisError> {
    if !configure_artifact.exists() {
        return Ok(false);
    }

    let configure_mtime = configure_artifact.metadata()?.modified()?;
    for input in [
        native_dir.join("CMakeLists.txt"),
        native_dir.join("mac").join("Info.plist.in"),
        native_dir.join("mac").join("helper-Info.plist.in"),
    ] {
        if input.exists() && input.metadata()?.modified()? > configure_mtime {
            return Ok(true);
        }
    }

    Ok(false)
}

fn build_dir(root: &Path, platform: NativePlatform) -> PathBuf {
    root.join("native").join("build").join(platform.as_str())
}

fn configure_artifact(root: &Path, platform: NativePlatform) -> PathBuf {
    let build_dir = build_dir(root, platform);
    match platform {
        NativePlatform::Macos => build_dir.join("aegis_native.xcodeproj"),
        NativePlatform::Linux => build_dir.join("CMakeCache.txt"),
    }
}

fn artifact_output_dir(
    root: &Path,
    platform: NativePlatform,
    configuration: NativeConfiguration,
) -> PathBuf {
    let build_dir = build_dir(root, platform);
    match platform {
        NativePlatform::Macos => build_dir.join(configuration.as_str()),
        NativePlatform::Linux => build_dir.join(configuration.as_str().to_ascii_lowercase()),
    }
}

fn cef_sdk_dir(platform: NativePlatform) -> &'static str {
    match platform {
        NativePlatform::Macos => {
            "third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_macosarm64"
        }
        NativePlatform::Linux => {
            "third_party/cef/cef_binary_146.0.6+g68649e2+chromium-146.0.7680.154_linux64"
        }
    }
}

fn preferred_app_dir(root: &Path, platform: NativePlatform) -> PathBuf {
    installed_app_dir(platform).unwrap_or_else(|| default_workspace_app_dir(root, platform))
}

fn default_workspace_app_dir(root: &Path, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => {
            artifact_for_target(root, NativeConfiguration::Release, DEFAULT_TARGET)
        }
        NativePlatform::Linux => root.join("native/build/linux/release"),
    }
}

fn workspace_runtime_is_current(
    root: &Path,
    workspace_app_executable: &Path,
    workspace_host_library: &Path,
) -> Result<bool, AegisError> {
    if !workspace_app_executable.exists() || !workspace_host_library.exists() {
        return Ok(false);
    }

    let app_mtime = workspace_app_executable.metadata()?.modified()?;
    let host_mtime = workspace_host_library.metadata()?.modified()?;
    let artifact_mtime = std::cmp::min(app_mtime, host_mtime);

    for input in workspace_runtime_inputs(root) {
        if input.exists() && input.metadata()?.modified()? > artifact_mtime {
            return Ok(false);
        }
    }

    Ok(true)
}

fn workspace_runtime_inputs(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("assets").join("js").join("aegis_runtime.js"),
        root.join("scripts").join("generate_runtime_header.py"),
        root.join("native").join("CMakeLists.txt"),
        root.join("native").join("aegis_bootstrap.h"),
        root.join("native").join("aegis_app.cc"),
        root.join("native").join("aegis_app.h"),
        root.join("native").join("aegis_client.cc"),
        root.join("native").join("src").join("aegis_cef_host.cpp"),
        root.join("native").join("src").join("aegis_protocol.cpp"),
        root.join("native")
            .join("include")
            .join("aegis_cef_host.hpp"),
        root.join("native")
            .join("include")
            .join("aegis_protocol.hpp"),
    ]
}

fn installed_app_dir(platform: NativePlatform) -> Option<PathBuf> {
    let path = canonical_install_dir(platform)?;
    path.exists().then_some(path)
}

fn canonical_install_dir(platform: NativePlatform) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(match platform {
        NativePlatform::Macos => PathBuf::from(home).join("Applications").join("Aegis.app"),
        NativePlatform::Linux => PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("aegis")
            .join("Aegis"),
    })
}

pub fn canonical_install_host_library() -> Option<PathBuf> {
    let platform = current_platform();
    canonical_install_dir(platform).map(|install_dir| bundled_host_library(&install_dir, platform))
}

fn workspace_host_library(root: &Path, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => artifact_output_dir(root, platform, NativeConfiguration::Release)
            .join("libaegis_host.dylib"),
        NativePlatform::Linux => artifact_output_dir(root, platform, NativeConfiguration::Release)
            .join("libaegis_host.so"),
    }
}

fn preferred_host_library(root: &Path, platform: NativePlatform, app_dir: &Path) -> PathBuf {
    match platform {
        NativePlatform::Macos => {
            let workspace = workspace_host_library(root, platform);
            if workspace.exists() {
                return workspace;
            }
            canonical_install_dir(platform)
                .map(|install_dir| bundled_host_library(&install_dir, platform))
                .unwrap_or_else(|| bundled_host_library(app_dir, platform))
        }
        NativePlatform::Linux => {
            let bundled = bundled_host_library(app_dir, platform);
            if bundled.exists() {
                return bundled;
            }
            workspace_host_library(root, platform)
        }
    }
}

fn bundled_host_library(app_dir: &Path, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => app_dir
            .join("Contents")
            .join("Frameworks")
            .join("libaegis_host.dylib"),
        NativePlatform::Linux => app_dir.join("lib").join("libaegis_host.so"),
    }
}

fn bundled_cli_path(app_dir: &Path, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => app_dir.join("Contents").join("MacOS").join("aegis_cli"),
        NativePlatform::Linux => app_dir.join("bin").join("aegis_cli"),
    }
}

fn canonical_app_executable_path(app_dir: &Path, platform: NativePlatform) -> PathBuf {
    match platform {
        NativePlatform::Macos => app_executable(app_dir, platform),
        NativePlatform::Linux => app_dir.join("bin").join(DEFAULT_TARGET),
    }
}

#[cfg(target_os = "macos")]
fn macos_bundle_executable_name(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents").join("Info.plist");
    let output = Command::new("defaults")
        .arg("read")
        .arg(plist)
        .arg("CFBundleExecutable")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(target_os = "macos")]
fn detect_macos_bundle_executable(bundle: &Path) -> Option<String> {
    let macos_dir = bundle.join("Contents").join("MacOS");
    let preferred = macos_dir.join(DEFAULT_TARGET);
    if preferred.exists() {
        return Some(DEFAULT_TARGET.to_string());
    }

    let entries = fs::read_dir(&macos_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name()?.to_string_lossy();
        if name == "aegis_cli" {
            continue;
        }
        return Some(name.into_owned());
    }

    None
}

#[cfg(not(target_os = "macos"))]
fn macos_bundle_executable_name(_bundle: &Path) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_bundle_executable(_bundle: &Path) -> Option<String> {
    None
}

fn copy_file_with_mode(source: &Path, target: &Path) -> Result<(), AegisError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)?;
    let permissions = fs::metadata(source)?.permissions();
    fs::set_permissions(target, permissions)?;
    Ok(())
}

fn copy_linux_runtime_artifacts(source_dir: &Path, target_dir: &Path) -> Result<(), AegisError> {
    fs::create_dir_all(target_dir)?;
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let should_copy = name == "libcef.so"
            || name == "chrome-sandbox"
            || name == "snapshot_blob.bin"
            || name == "v8_context_snapshot.bin"
            || name == "icudtl.dat"
            || name == "vk_swiftshader_icd.json"
            || name.ends_with(".pak")
            || name == "locales"
            || name == "swiftshader";
        if !should_copy {
            continue;
        }
        let target = target_dir.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            copy_file_with_mode(&path, &target)?;
        }
    }
    let helper = source_dir.join("aegis_helper");
    if helper.exists() {
        copy_file_with_mode(&helper, &target_dir.join("aegis_helper"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn clear_quarantine_attribute(bundle: &Path) {
    let _ = Command::new("xattr").args(["-cr"]).arg(bundle).output();
}

#[cfg(not(target_os = "macos"))]
fn clear_quarantine_attribute(_bundle: &Path) {}

#[cfg(target_os = "macos")]
fn load_code_signing_config() -> CodeSigningConfig {
    const DEFAULT_CODESIGN_IDENTITY: &str = "-";
    const DEFAULT_CODESIGN_OPTIONS: &str = "runtime";

    let identity = std::env::var("AEGIS_CODESIGN_IDENTITY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CODESIGN_IDENTITY.to_string());
    let options = std::env::var("AEGIS_CODESIGN_OPTIONS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            (identity != DEFAULT_CODESIGN_IDENTITY).then(|| DEFAULT_CODESIGN_OPTIONS.into())
        });
    let entitlements = std::env::var_os("AEGIS_CODESIGN_ENTITLEMENTS")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);

    CodeSigningConfig {
        identity,
        options,
        entitlements,
    }
}

#[cfg(target_os = "macos")]
fn sign_bundle_for_distribution(bundle: &Path) -> Result<(), AegisError> {
    let config = load_code_signing_config();
    for nested_code in nested_code_targets(bundle)? {
        sign_path(&nested_code, &config, false)?;
    }
    sign_path(bundle, &config, true)
}

#[cfg(not(target_os = "macos"))]
fn sign_bundle_for_distribution(_bundle: &Path) -> Result<(), AegisError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn nested_code_targets(bundle: &Path) -> Result<Vec<PathBuf>, AegisError> {
    let frameworks_dir = bundle.join("Contents").join("Frameworks");
    let macos_dir = bundle.join("Contents").join("MacOS");
    let mut targets = Vec::new();

    if frameworks_dir.exists() {
        for entry in fs::read_dir(&frameworks_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_nested_bundle =
                path.is_dir() && (name.ends_with(".app") || name.ends_with(".framework"));
            let is_nested_library = path.is_file() && name.ends_with(".dylib");
            if is_nested_bundle || is_nested_library {
                targets.push(path);
            }
        }
    }

    let bundled_cli = macos_dir.join("aegis_cli");
    if bundled_cli.exists() {
        targets.push(bundled_cli);
    }

    Ok(targets)
}

#[cfg(target_os = "macos")]
fn sign_path(
    path: &Path,
    config: &CodeSigningConfig,
    apply_entitlements: bool,
) -> Result<(), AegisError> {
    let mut args = vec![
        "--force".to_string(),
        "--sign".to_string(),
        config.identity.clone(),
        "--timestamp=none".to_string(),
    ];
    if let Some(options) = &config.options {
        args.push("--options".to_string());
        args.push(options.clone());
    }
    if apply_entitlements && let Some(entitlements) = &config.entitlements {
        args.push("--entitlements".to_string());
        args.push(path_str(entitlements)?.to_string());
    }
    args.push(path_str(path)?.to_string());
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_checked("codesign", &borrowed, Path::new("/"))
}

#[cfg(target_os = "macos")]
fn verify_signed_bundle(bundle: &Path) -> Result<(), AegisError> {
    run_checked(
        "codesign",
        &["--verify", "--strict", "--verbose=2", path_str(bundle)?],
        Path::new("/"),
    )?;

    let signing = load_code_signing_config();
    if signing.identity != "-" {
        run_checked(
            "spctl",
            &[
                "--assess",
                "--type",
                "execute",
                "--verbose=4",
                path_str(bundle)?,
            ],
            Path::new("/"),
        )?;
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn verify_signed_bundle(_bundle: &Path) -> Result<(), AegisError> {
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), AegisError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&source_path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &target_path)?;
        } else {
            copy_file_with_mode(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn run_checked(program: &str, args: &[&str], root: &Path) -> Result<(), AegisError> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(AegisError::Bridge(format!(
        "{program} failed with status {}: {}{}{}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stdout.trim(),
        if !stdout.trim().is_empty() && !stderr.trim().is_empty() {
            " | "
        } else {
            ""
        },
        stderr.trim()
    )))
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {} >/dev/null 2>&1", name)])
        .status()
        .is_ok_and(|status| status.success())
}

fn required_command_names(platform: NativePlatform) -> Vec<&'static str> {
    match platform {
        NativePlatform::Macos => vec!["cargo", "cmake", "python3", "xcodebuild", "codesign"],
        NativePlatform::Linux => vec!["cargo", "cmake", "python3"],
    }
}

fn apple_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    }
}

fn path_str(path: &Path) -> Result<&str, AegisError> {
    path.to_str().ok_or_else(path_encoding_error)
}

fn path_encoding_error() -> AegisError {
    AegisError::Bridge("path is not valid utf-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn home_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn linux_status_falls_back_to_workspace_host_library_without_installed_bundle() {
        let _guard = home_env_lock()
            .lock()
            .expect("home env lock should not poison");
        let temp = tempfile::tempdir().expect("temporary dir should be created");
        let repo_root = temp.path().join("repo");
        let workspace_host = workspace_host_library(&repo_root, NativePlatform::Linux);

        fs::create_dir_all(
            workspace_host
                .parent()
                .expect("workspace host library should have a parent"),
        )
        .expect("workspace host dir should be created");
        fs::write(&workspace_host, b"host").expect("workspace host should be created");

        unsafe {
            std::env::remove_var("HOME");
        }

        let status = status(&repo_root);
        if current_platform() == NativePlatform::Linux {
            assert_eq!(status.default_host_library, workspace_host);
            assert!(status.default_host_library_present);
        }
    }

    #[test]
    fn linux_doctor_reports_expected_canonical_install_paths() {
        let _guard = home_env_lock()
            .lock()
            .expect("home env lock should not poison");
        let temp = tempfile::tempdir().expect("temporary dir should be created");
        let repo_root = temp.path().join("repo");
        let home_dir = temp.path().join("home");

        fs::create_dir_all(&repo_root).expect("repo root should be created");
        unsafe {
            std::env::set_var("HOME", &home_dir);
        }

        let doctor = doctor(&repo_root);
        if current_platform() == NativePlatform::Linux {
            assert_eq!(
                doctor.canonical_install_dir,
                Some(
                    home_dir
                        .join(".local")
                        .join("share")
                        .join("aegis")
                        .join("Aegis")
                )
            );
            assert_eq!(
                doctor.canonical_install_host_library,
                Some(
                    home_dir
                        .join(".local")
                        .join("share")
                        .join("aegis")
                        .join("Aegis")
                        .join("lib")
                        .join("libaegis_host.so")
                )
            );
            assert_eq!(
                doctor.canonical_install_cli,
                Some(
                    home_dir
                        .join(".local")
                        .join("share")
                        .join("aegis")
                        .join("Aegis")
                        .join("bin")
                        .join("aegis_cli")
                )
            );
        }

        unsafe {
            std::env::remove_var("HOME");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_status_prefers_workspace_host_library_for_runtime() {
        let _guard = home_env_lock()
            .lock()
            .expect("home env lock should not poison");
        let temp = tempfile::tempdir().expect("temporary dir should be created");
        let repo_root = temp.path().join("repo");
        let home_dir = temp.path().join("home");
        let install_dir = home_dir.join("Applications").join("Aegis.app");
        let installed_host = bundled_host_library(&install_dir, NativePlatform::Macos);
        let workspace_host = workspace_host_library(&repo_root, NativePlatform::Macos);

        fs::create_dir_all(
            installed_host
                .parent()
                .expect("host library should have a parent"),
        )
        .expect("bundle framework dir should be created");
        fs::write(&installed_host, b"host").expect("bundled host should be created");
        fs::create_dir_all(&repo_root).expect("repo root should be created");
        fs::create_dir_all(
            workspace_host
                .parent()
                .expect("workspace host library should have a parent"),
        )
        .expect("workspace host dir should be created");
        fs::write(&workspace_host, b"host").expect("workspace host should be created");

        unsafe {
            std::env::set_var("HOME", &home_dir);
        }

        let status = status(&repo_root);
        assert_eq!(status.default_host_library, workspace_host);
        assert!(status.default_host_library_present);

        unsafe {
            std::env::remove_var("HOME");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_status_falls_back_to_installed_host_library_when_workspace_missing() {
        let _guard = home_env_lock()
            .lock()
            .expect("home env lock should not poison");
        let temp = tempfile::tempdir().expect("temporary dir should be created");
        let repo_root = temp.path().join("repo");
        let home_dir = temp.path().join("home");
        let install_dir = home_dir.join("Applications").join("Aegis.app");
        let expected_host = bundled_host_library(&install_dir, NativePlatform::Macos);
        let workspace_host = workspace_host_library(&repo_root, NativePlatform::Macos);

        fs::create_dir_all(&repo_root).expect("repo root should be created");
        fs::create_dir_all(
            expected_host
                .parent()
                .expect("installed host library should have a parent"),
        )
        .expect("installed host dir should be created");
        fs::write(&expected_host, b"host").expect("installed host should be created");
        assert!(!workspace_host.exists());

        unsafe {
            std::env::set_var("HOME", &home_dir);
        }

        let status = status(&repo_root);
        assert_eq!(status.default_host_library, expected_host);
        assert!(status.default_host_library_present);

        unsafe {
            std::env::remove_var("HOME");
        }
    }
}
