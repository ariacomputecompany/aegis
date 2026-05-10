#include "include/aegis_platform.h"

#include "include/cef_app.h"

#include <sys/syscall.h>
#include <unistd.h>

#include <stdexcept>

namespace {

std::filesystem::path DetectInstallRoot(const std::filesystem::path& library_dir) {
  if (library_dir.filename() == "lib") {
    return library_dir.parent_path();
  }
  return library_dir;
}

}  // namespace

int AegisPlatformRunMain(AegisPlatformMainEntry entry, int argc, char* argv[]) {
  return entry(argc, argv);
}

bool AegisPlatformIsMainThread() {
  return static_cast<pid_t>(syscall(SYS_gettid)) == getpid();
}

void AegisPlatformInitializeMainApplication(bool embedded_command_mode) {
  static_cast<void>(embedded_command_mode);
}

void AegisPlatformConfigureActivation(bool embedded_command_mode, bool headful_mode) {
  static_cast<void>(embedded_command_mode);
  static_cast<void>(headful_mode);
}

void AegisInstallModalAlertSuppression() {}

void AegisInitializeBrowserHostApplication() {}

bool AegisPlatformLoadCefRuntime(const std::filesystem::path& cef_library,
                                 std::string* error) {
  static_cast<void>(cef_library);
  static_cast<void>(error);
  return true;
}

void AegisPlatformUnloadCefRuntime() {}

void AegisConfigureCefSettings(const AegisCefBootstrapOptions& options,
                               CefSettings* settings) {
  if (settings == nullptr) {
    return;
  }
#if !defined(CEF_USE_SANDBOX)
  settings->no_sandbox = true;
#endif
  settings->windowless_rendering_enabled = true;
  settings->command_line_args_disabled = false;
  settings->external_message_pump = options.external_message_pump;

  if (!options.browser_subprocess_path.empty()) {
    CefString(&settings->browser_subprocess_path) = options.browser_subprocess_path;
  }
  if (!options.resources_dir_path.empty()) {
    CefString(&settings->resources_dir_path) = options.resources_dir_path;
  }
  if (!options.locales_dir_path.empty()) {
    CefString(&settings->locales_dir_path) = options.locales_dir_path;
  }
  if (!options.root_cache_path.empty()) {
    CefString(&settings->root_cache_path) = options.root_cache_path;
  }
  if (!options.cache_path.empty()) {
    CefString(&settings->cache_path) = options.cache_path;
  }
}

bool AegisExecuteProcessAndInitialize(const CefMainArgs& main_args,
                                      const AegisCefBootstrapOptions& options,
                                      CefRefPtr<CefApp> app,
                                      int* subprocess_exit_code,
                                      std::string* error) {
  if (options.initialize_browser_host_application) {
    AegisInitializeBrowserHostApplication();
  }

  CefSettings settings;
  AegisConfigureCefSettings(options, &settings);

  const int execute_process_result = CefExecuteProcess(main_args, app.get(), nullptr);
  if (subprocess_exit_code != nullptr) {
    *subprocess_exit_code = execute_process_result;
  }
  if (execute_process_result >= 0) {
    return false;
  }
  if (!CefInitialize(main_args, settings, app.get(), nullptr)) {
    if (error != nullptr) {
      *error = "CefInitialize failed";
    }
    return false;
  }
  return true;
}

AegisPlatformPaths AegisResolvePlatformPaths(
    const std::filesystem::path& library_dir) {
  const auto app_root = DetectInstallRoot(library_dir);
  AegisPlatformPaths paths{
      .library_dir = library_dir,
      .app_root = app_root,
      .main_executable = app_root / "bin" / "aegis_native",
      .helper_executable = library_dir / "aegis_helper",
      .cef_library = library_dir / "libcef.so",
      .framework_dir = {},
      .resources_dir = library_dir,
      .locales_dir = library_dir / "locales",
      .main_bundle_path = {},
  };

  if (!std::filesystem::exists(paths.main_executable)) {
    const auto workspace_binary = app_root / "aegis_native";
    if (std::filesystem::exists(workspace_binary)) {
      paths.main_executable = workspace_binary;
    } else {
      throw std::runtime_error("aegis_native is missing; build the native runtime first");
    }
  }
  if (!std::filesystem::exists(paths.helper_executable)) {
    const auto workspace_helper = app_root / "aegis_helper";
    if (std::filesystem::exists(workspace_helper)) {
      paths.helper_executable = workspace_helper;
    } else {
      throw std::runtime_error("aegis_helper is missing; build the native helper first");
    }
  }
  if (!std::filesystem::exists(paths.cef_library)) {
    throw std::runtime_error("libcef.so is missing next to the host library");
  }
  if (!std::filesystem::exists(paths.locales_dir)) {
    throw std::runtime_error("CEF locales are missing from the Linux runtime resources");
  }

  return paths;
}

bool AegisUseExternalBrowserHostWindow() { return false; }

void AegisPlatformConfigureTopLevelWindow(CefWindowInfo* window_info,
                                          const std::string& title,
                                          int width,
                                          int height) {
  if (window_info == nullptr) {
    return;
  }
  CefString(&window_info->window_name) = title;
  window_info->bounds = CefRect(0, 0, width, height);
}

CefWindowHandle AegisCreateBrowserHostView(const std::string& title,
                                           int width,
                                           int height) {
  static_cast<void>(title);
  static_cast<void>(width);
  static_cast<void>(height);
  return kNullWindowHandle;
}

void AegisShowBrowserHostWindow() {}

void AegisSetBrowserHostTitle(const std::string& title) {
  static_cast<void>(title);
}

void AegisSetBrowserHostAddress(const std::string& url) {
  static_cast<void>(url);
}

void AegisSetBrowserHostNavigationState(bool can_go_back,
                                        bool can_go_forward,
                                        bool is_loading) {
  static_cast<void>(can_go_back);
  static_cast<void>(can_go_forward);
  static_cast<void>(is_loading);
}

void AegisAttachBrowserToHostWindow(CefRefPtr<CefBrowser> browser) {
  static_cast<void>(browser);
}

void AegisCloseBrowserHostWindow() {}

void AegisPumpBrowserHostWindow() {}

bool AegisBrowserHostWindowCloseRequested() { return false; }
