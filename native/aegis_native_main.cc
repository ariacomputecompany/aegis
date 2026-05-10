#include "aegis_app.h"
#include "include/aegis_cef_host.hpp"
#include "include/aegis_platform.h"
#include "include/base/cef_logging.h"
#include "include/cef_browser.h"
#include "include/wrapper/cef_library_loader.h"

#include <cstdlib>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

namespace {

int RunAegisMain(int argc, char* argv[]) {
  CefScopedLibraryLoader loader;
  if (!loader.LoadInMain()) {
    return 1;
  }

  CefMainArgs main_args(argc, argv);

  std::string config_path;
  std::string request_path;
  std::string response_path;
  std::string error_path;
  std::string debug_log_path_arg;
  std::string startup_url;
  bool headful_mode = false;
  int operation_value = 0;
  for (int i = 1; i < argc; ++i) {
    const std::string arg(argv[i]);
    if (arg == "--mode" && i + 1 < argc) {
      headful_mode = std::string(argv[++i]) == "headful";
      continue;
    }
    if (arg == "--aegis-config" && i + 1 < argc) {
      config_path = argv[++i];
    } else if (arg == "--aegis-request" && i + 1 < argc) {
      request_path = argv[++i];
    } else if (arg == "--aegis-response" && i + 1 < argc) {
      response_path = argv[++i];
    } else if (arg == "--aegis-error" && i + 1 < argc) {
      error_path = argv[++i];
    } else if (arg == "--aegis-debug-log" && i + 1 < argc) {
      debug_log_path_arg = argv[++i];
    } else if (arg == "--url" && i + 1 < argc) {
      startup_url = argv[++i];
    } else if (arg == "--aegis-op" && i + 1 < argc) {
      operation_value = std::stoi(argv[++i]);
    }
  }
  const bool embedded_command_mode = !config_path.empty() && !request_path.empty() &&
                                     !response_path.empty() && operation_value != 0;
  const auto response_dir_end = response_path.find_last_of('/');
  const auto debug_log_path = !debug_log_path_arg.empty()
                                  ? debug_log_path_arg
                                  : (embedded_command_mode
                                         ? ((response_dir_end == std::string::npos
                                                 ? std::string(".")
                                                 : response_path.substr(0, response_dir_end)) +
                                            "/debug.log")
                                         : std::string());
  auto append_debug = [&debug_log_path](const std::string& message) {
    if (debug_log_path.empty()) {
      return;
    }
    std::ofstream output(debug_log_path, std::ios::app);
    if (!output.is_open()) {
      return;
    }
    output << message << '\n';
  };

  AegisPlatformInitializeMainApplication(embedded_command_mode);

  if (embedded_command_mode) {
    std::ofstream(debug_log_path, std::ios::trunc).close();
    unsetenv("AEGIS_DEBUG_LOG");
    setenv("AEGIS_DEBUG_LOG", debug_log_path.c_str(), 1);
    append_debug("main: embedded command mode");
  }

  CefRefPtr<AegisApp> app(new AegisApp(!embedded_command_mode, startup_url));
  AegisCefBootstrapOptions bootstrap_options;
  bootstrap_options.headless = embedded_command_mode && !headful_mode;
  bootstrap_options.external_message_pump = embedded_command_mode;
  int subprocess_exit_code = -1;
  std::string initialize_error;
  append_debug("main: before canonical cef bootstrap");
  const bool initialized = AegisExecuteProcessAndInitialize(
      main_args, bootstrap_options, app, &subprocess_exit_code, &initialize_error);
  append_debug("main: after canonical cef bootstrap subprocess_exit_code=" +
               std::to_string(subprocess_exit_code));
  if (subprocess_exit_code >= 0) {
    return subprocess_exit_code;
  }
  if (!initialized) {
    append_debug("main: " + initialize_error);
    return CefGetExitCode();
  }
  append_debug("main: after canonical cef bootstrap");

  AegisPlatformConfigureActivation(embedded_command_mode, headful_mode);

  if (embedded_command_mode) {
    auto read_file = [](const std::string& path, std::vector<std::uint8_t>* out) -> bool {
      std::ifstream input(path, std::ios::binary);
      if (!input.is_open()) {
        return false;
      }
      *out = std::vector<std::uint8_t>((std::istreambuf_iterator<char>(input)),
                                       std::istreambuf_iterator<char>());
      return true;
    };
    auto write_file = [](const std::string& path, const std::string& value) -> bool {
      std::ofstream output(path, std::ios::binary | std::ios::trunc);
      if (!output.is_open()) {
        return false;
      }
      output.write(value.data(), static_cast<std::streamsize>(value.size()));
      return output.good();
    };
    auto write_bytes = [](const std::string& path, const std::vector<std::uint8_t>& bytes) -> bool {
      std::ofstream output(path, std::ios::binary | std::ios::trunc);
      if (!output.is_open()) {
        return false;
      }
      if (!bytes.empty()) {
        output.write(reinterpret_cast<const char*>(bytes.data()),
                     static_cast<std::streamsize>(bytes.size()));
      }
      return output.good();
    };

    int exit_code = 0;
    std::vector<std::uint8_t> config_bytes;
    std::vector<std::uint8_t> request_bytes;
    if (!read_file(config_path, &config_bytes)) {
      if (!error_path.empty()) {
        write_file(error_path, "failed to read config file");
      }
      exit_code = 3;
    } else if (!read_file(request_path, &request_bytes)) {
      if (!error_path.empty()) {
        write_file(error_path, "failed to read request file");
      }
      exit_code = 3;
    } else {
      append_debug("main: before embedded host operation");
      std::vector<std::uint8_t> response;
      std::string error;
      const bool ok = aegis::RunEmbeddedHostOperation(
          config_bytes,
          static_cast<aegis::EmbeddedHostOperation>(operation_value),
          request_bytes,
          &response,
          &error);
      append_debug(std::string("main: embedded host operation result=") +
                   (ok ? "ok" : "error"));
      if (!ok) {
        if (!error_path.empty()) {
          write_file(error_path, error);
        }
        exit_code = 2;
      } else if (!write_bytes(response_path, response)) {
        if (!error_path.empty()) {
          write_file(error_path, "failed to write response file");
        }
        exit_code = 3;
      }
    }

    append_debug("main: embedded command complete");
    std::fflush(nullptr);
    std::_Exit(exit_code);
  }

  CefRunMessageLoop();
  CefShutdown();
  return 0;
}

}  // namespace

int main(int argc, char* argv[]) {
  return AegisPlatformRunMain(&RunAegisMain, argc, argv);
}
