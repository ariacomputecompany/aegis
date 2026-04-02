#include "aegis_cef_host.hpp"

#include <dlfcn.h>

#include <chrono>
#include <cctype>
#include <condition_variable>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <fstream>
#include <functional>
#include <cstdlib>
#include <cerrno>
#include <limits>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <pthread.h>
#include <signal.h>
#include <stdexcept>
#include <string>
#include <thread>
#include <unistd.h>
#include <utility>
#include <vector>

#include "../aegis_app.h"
#include "../aegis_client.h"
#include "../aegis_messages.h"
#include "../aegis_state_paths.h"
#include "../include/aegis_platform.h"
#include "include/base/cef_bind.h"
#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/cef_cookie.h"
#include "include/cef_parser.h"
#include "include/cef_preference.h"
#include "include/cef_request_context.h"
#include "include/cef_waitable_event.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_window.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace aegis {
namespace {

thread_local std::string g_last_host_error;

constexpr auto kStartupTimeout = std::chrono::seconds(30);
constexpr auto kRendererTimeout = std::chrono::seconds(30);
constexpr auto kShutdownTimeout = std::chrono::seconds(2);
constexpr auto kPumpInterval = std::chrono::milliseconds(10);
constexpr char kBootstrapUrl[] =
    "data:text/html,%3C!doctype%20html%3E%3Chtml%3E%3Chead%3E%3Cmeta%20charset%3D%22utf-8%22%3E%3C%2Fhead%3E%3Cbody%3E%3C%2Fbody%3E%3C%2Fhtml%3E";

void AppendDebugLog(const std::string& message);

std::string UrlRequestStatusToString(cef_urlrequest_status_t status) {
  switch (status) {
    case UR_UNKNOWN:
      return "unknown";
    case UR_SUCCESS:
      return "success";
    case UR_IO_PENDING:
      return "io_pending";
    case UR_CANCELED:
      return "canceled";
    case UR_FAILED:
      return "failed";
  }
  return "unknown";
}

std::string EscapeJsonString(const std::string& input) {
  std::string output;
  output.reserve(input.size());
  for (char ch : input) {
    switch (ch) {
      case '\\':
        output += "\\\\";
        break;
      case '"':
        output += "\\\"";
        break;
      case '\n':
        output += "\\n";
        break;
      case '\r':
        output += "\\r";
        break;
      case '\t':
        output += "\\t";
        break;
      default:
        output.push_back(ch);
        break;
    }
  }
  return output;
}

bool IsStructuredOperationError(const std::string& message) {
  return message.find("\"kind\":\"operation_error\"") != std::string::npos;
}

void ApplyBooleanPreference(CefRefPtr<CefPreferenceManager> manager,
                            const char* name,
                            bool value) {
  if (!manager.get() || !manager->HasPreference(name) ||
      !manager->CanSetPreference(name)) {
    return;
  }

  auto pref_value = CefValue::Create();
  pref_value->SetBool(value);
  CefString error;
  if (!manager->SetPreference(name, pref_value, error)) {
    AppendDebugLog(std::string("host: failed_to_set_preference ") + name + " " +
                   error.ToString());
  }
}

void ApplyAegisProductionPreferences(CefRefPtr<CefPreferenceManager> manager) {
  ApplyBooleanPreference(manager, "credentials_enable_service", false);
  ApplyBooleanPreference(manager, "profile.password_manager_enabled", false);
  ApplyBooleanPreference(manager, "profile.password_manager_leak_detection", false);
  ApplyBooleanPreference(manager, "autofill.profile_enabled", false);
  ApplyBooleanPreference(manager, "autofill.credit_card_enabled", false);
}

void AppendDebugLog(const std::string& message) {
  const char* path = std::getenv("AEGIS_DEBUG_LOG");
  if (path == nullptr || *path == '\0') {
    return;
  }
  static const auto start = std::chrono::steady_clock::now();
  std::ofstream output(path, std::ios::app);
  if (!output.is_open()) {
    return;
  }
  const auto elapsed_ms =
      std::chrono::duration_cast<std::chrono::milliseconds>(
          std::chrono::steady_clock::now() - start)
          .count();
  output << "[" << elapsed_ms << "ms] " << message << '\n';
}

std::vector<std::uint8_t> CopyInput(const std::uint8_t* input_ptr, std::size_t input_len) {
  if (input_ptr == nullptr || input_len == 0) {
    return {};
  }
  return {input_ptr, input_ptr + input_len};
}

void WriteOutput(std::vector<std::uint8_t> bytes, AegisHostBuffer* output) {
  if (output == nullptr) {
    throw std::runtime_error("output buffer is null");
  }

  if (bytes.empty()) {
    output->ptr = nullptr;
    output->len = 0;
    return;
  }

  auto* heap = new std::uint8_t[bytes.size()];
  std::copy(bytes.begin(), bytes.end(), heap);
  output->ptr = heap;
  output->len = bytes.size();
}

CefRefPtr<CefDictionaryValue> RequireDictionary(CefRefPtr<CefValue> value,
                                                const char* message) {
  if (!value.get() || value->GetType() != VTYPE_DICTIONARY) {
    throw std::runtime_error(message);
  }
  return value->GetDictionary()->Copy(false);
}

CefRefPtr<CefValue> ParseJsonValue(const std::string& json, const char* message) {
  auto value = CefParseJSON(json, JSON_PARSER_RFC);
  if (!value.get()) {
    throw std::runtime_error(message);
  }
  return value;
}

std::string WriteJson(CefRefPtr<CefValue> value) {
  return CefWriteJSON(value, JSON_WRITER_DEFAULT).ToString();
}

std::string WriteJson(CefRefPtr<CefDictionaryValue> value) {
  auto wrapped = CefValue::Create();
  wrapped->SetDictionary(value);
  return WriteJson(wrapped);
}

std::filesystem::path LibraryDirectory() {
  Dl_info info{};
  if (dladdr(reinterpret_cast<void*>(&aegis_get_function_table), &info) == 0 ||
      info.dli_fname == nullptr) {
    throw std::runtime_error("failed to resolve host library path");
  }
  return std::filesystem::path(info.dli_fname).parent_path();
}

struct ManagedCookie {
  std::string name;
  std::string value;
  std::string domain;
  std::string path = "/";
  std::optional<std::uint64_t> expires_unix;
  bool secure = false;
  bool http_only = false;
};


std::string TrimAscii(std::string value) {
  while (!value.empty() &&
         std::isspace(static_cast<unsigned char>(value.front())) != 0) {
    value.erase(value.begin());
  }
  while (!value.empty() &&
         std::isspace(static_cast<unsigned char>(value.back())) != 0) {
    value.pop_back();
  }
  return value;
}

bool CaseEqualAscii(const std::string& left, const std::string& right) {
  if (left.size() != right.size()) {
    return false;
  }
  for (std::size_t index = 0; index < left.size(); ++index) {
    if (std::tolower(static_cast<unsigned char>(left[index])) !=
        std::tolower(static_cast<unsigned char>(right[index]))) {
      return false;
    }
  }
  return true;
}

std::optional<std::string> UrlScheme(const std::string& url) {
  CefURLParts parts;
  if (!CefParseURL(url, parts)) {
    return std::nullopt;
  }
  return CefString(&parts.scheme).ToString();
}

std::optional<std::string> UrlHost(const std::string& url) {
  CefURLParts parts;
  if (!CefParseURL(url, parts)) {
    return std::nullopt;
  }
  return CefString(&parts.host).ToString();
}

std::optional<std::string> UrlPath(const std::string& url) {
  CefURLParts parts;
  if (!CefParseURL(url, parts)) {
    return std::nullopt;
  }
  const auto path = CefString(&parts.path).ToString();
  if (path.empty()) {
    return std::string("/");
  }
  return path;
}

struct HostPaths {
  std::filesystem::path library_dir;
  std::filesystem::path app_root;
  std::filesystem::path main_executable;
  std::filesystem::path helper_executable;
  std::filesystem::path cef_library;
  std::filesystem::path framework_dir;
  std::filesystem::path resources_dir;
  std::filesystem::path locales_dir;
  std::filesystem::path main_bundle_path;
};

HostPaths ResolveHostPaths() {
  const auto anchor_dir = LibraryDirectory();
  const auto resolved = AegisResolvePlatformPaths(anchor_dir);
  return HostPaths{
      .library_dir = resolved.library_dir,
      .app_root = resolved.app_root,
      .main_executable = resolved.main_executable,
      .helper_executable = resolved.helper_executable,
      .cef_library = resolved.cef_library,
      .framework_dir = resolved.framework_dir,
      .resources_dir = resolved.resources_dir,
      .locales_dir = resolved.locales_dir,
      .main_bundle_path = resolved.main_bundle_path,
  };
}

struct BrowserOptions {
  bool headless = true;
  std::string start_url = kBootstrapUrl;
};

BrowserOptions ParseBrowserOptions(const std::vector<std::uint8_t>& bytes) {
  BrowserOptions options;
  if (bytes.empty()) {
    return options;
  }

  const auto json = std::string(bytes.begin(), bytes.end());

  auto skip_whitespace = [&](std::size_t* index) {
    while (*index < json.size() &&
           std::isspace(static_cast<unsigned char>(json[*index])) != 0) {
      ++(*index);
    }
  };

  std::function<std::string(std::size_t*)> parse_string = [&](std::size_t* index) {
    if (*index >= json.size() || json[*index] != '"') {
      throw std::runtime_error("browser config is not valid json");
    }
    ++(*index);
    std::string value;
    while (*index < json.size()) {
      const char ch = json[*index];
      ++(*index);
      if (ch == '"') {
        return value;
      }
      if (ch != '\\') {
        value.push_back(ch);
        continue;
      }
      if (*index >= json.size()) {
        throw std::runtime_error("browser config is not valid json");
      }
      const char escaped = json[*index];
      ++(*index);
      switch (escaped) {
        case '"':
        case '\\':
        case '/':
          value.push_back(escaped);
          break;
        case 'b':
          value.push_back('\b');
          break;
        case 'f':
          value.push_back('\f');
          break;
        case 'n':
          value.push_back('\n');
          break;
        case 'r':
          value.push_back('\r');
          break;
        case 't':
          value.push_back('\t');
          break;
        case 'u':
          throw std::runtime_error("unicode escapes are not supported in browser config");
        default:
          throw std::runtime_error("browser config is not valid json");
      }
    }
    throw std::runtime_error("browser config is not valid json");
  };

  std::size_t index = 0;
  skip_whitespace(&index);
  if (index >= json.size() || json[index] != '{') {
    throw std::runtime_error("browser config must be a dictionary");
  }
  ++index;

  while (true) {
    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }
    if (json[index] == '}') {
      ++index;
      break;
    }

    const auto key = parse_string(&index);
    skip_whitespace(&index);
    if (index >= json.size() || json[index] != ':') {
      throw std::runtime_error("browser config is not valid json");
    }
    ++index;
    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }

    if (json[index] == '"') {
      const auto value = parse_string(&index);
      if (key == "mode") {
        options.headless = value != "headful";
      } else if (key == "start_url") {
        if (!value.empty()) {
          options.start_url = value;
        }
      }
    } else if (json.compare(index, 4, "null") == 0) {
      index += 4;
    } else {
      throw std::runtime_error("browser config contains unsupported value type");
    }

    skip_whitespace(&index);
    if (index >= json.size()) {
      throw std::runtime_error("browser config is not valid json");
    }
    if (json[index] == ',') {
      ++index;
      continue;
    }
    if (json[index] == '}') {
      ++index;
      break;
    }
    throw std::runtime_error("browser config is not valid json");
  }

  return options;
}

std::string CookieUrl(const CefRefPtr<CefDictionaryValue>& cookie) {
  auto domain = cookie->GetString("domain").ToString();
  while (!domain.empty() && domain.front() == '.') {
    domain.erase(domain.begin());
  }
  const auto path = cookie->HasKey("path") ? cookie->GetString("path").ToString() : std::string("/");
  const auto secure = cookie->HasKey("secure") && cookie->GetBool("secure");
  return (secure ? "https://" : "http://") + domain + (path.empty() ? "/" : path);
}

class CompletionSignal : public CefCompletionCallback {
 public:
  explicit CompletionSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete() override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(CompletionSignal);
};

class SetCookieSignal : public CefSetCookieCallback {
 public:
  explicit SetCookieSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete(bool) override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(SetCookieSignal);
};

class DeleteCookieSignal : public CefDeleteCookiesCallback {
 public:
  explicit DeleteCookieSignal(CefRefPtr<CefWaitableEvent> event) : event_(event) {}

  void OnComplete(int) override { event_->Signal(); }

 private:
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(DeleteCookieSignal);
};

class CookieCollector : public CefCookieVisitor {
 public:
  CookieCollector(std::vector<CefCookie>* cookies, CefRefPtr<CefWaitableEvent> event)
      : cookies_(cookies), event_(event) {}

  ~CookieCollector() override { event_->Signal(); }

  bool Visit(const CefCookie& cookie, int, int, bool&) override {
    cookies_->push_back(cookie);
    return true;
  }

 private:
  std::vector<CefCookie>* cookies_;
  CefRefPtr<CefWaitableEvent> event_;

  IMPLEMENT_REFCOUNTING(CookieCollector);
};

class HostWindowDelegate : public CefWindowDelegate {
 public:
  explicit HostWindowDelegate(CefRefPtr<CefBrowserView> browser_view)
      : browser_view_(browser_view) {}

  void OnWindowCreated(CefRefPtr<CefWindow> window) override {
    window->AddChildView(browser_view_);
    window->Show();
  }

  bool CanClose(CefRefPtr<CefWindow>) override { return true; }

 private:
  CefRefPtr<CefBrowserView> browser_view_;

  IMPLEMENT_REFCOUNTING(HostWindowDelegate);
};

class UiClosureTask : public CefTask {
 public:
  UiClosureTask(std::function<void()> work,
                std::shared_ptr<std::exception_ptr> error,
                CefRefPtr<CefWaitableEvent> done)
      : work_(std::move(work)), error_(std::move(error)), done_(done) {}

  void Execute() override {
    try {
      work_();
    } catch (...) {
      *error_ = std::current_exception();
    }
    done_->Signal();
  }

 private:
  std::function<void()> work_;
  std::shared_ptr<std::exception_ptr> error_;
  CefRefPtr<CefWaitableEvent> done_;

  IMPLEMENT_REFCOUNTING(UiClosureTask);
};

struct RendererReply {
  bool ok = false;
  std::string body;
};

class AegisCefHost;

struct RequestTelemetryState {
  std::string method;
  std::string url;
  bool is_main_frame = false;
  std::chrono::steady_clock::time_point started_at = std::chrono::steady_clock::now();
};

class OperationScope {
 public:
  OperationScope(AegisCefHost* host, std::string name);
  ~OperationScope();

 private:
  AegisCefHost* host_;
};

class AegisHostClient final : public AegisClient {
 public:
  AegisHostClient(bool headless,
                  ::AegisClientDelegate* delegate,
                  AegisCefHost* host)
      : AegisClient(headless, delegate), host_(host) {}

  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefProcessId source_process,
                                CefRefPtr<CefProcessMessage> message) override;

 private:
  AegisCefHost* host_;

  IMPLEMENT_REFCOUNTING(AegisHostClient);
};

class AegisCefHost final : public CefHost, public ::AegisClientDelegate {
 public:
  friend class OperationScope;

  explicit AegisCefHost(BrowserOptions options, bool manage_cef_lifecycle = true)
      : options_(std::move(options)),
        paths_(ResolveHostPaths()),
        runtime_session_paths_(AegisCreateRuntimeSessionPaths(
            options_.headless ? "serve-headless" : "serve-headful")),
        owner_thread_id_(std::this_thread::get_id()),
        manage_cef_lifecycle_(manage_cef_lifecycle) {
    if (!AegisPlatformIsMainThread()) {
      throw std::runtime_error("aegis CEF host must be created on the process main thread");
    }
    AppendDebugLog("host: constructed");
    if (manage_cef_lifecycle_) {
      Start();
    } else {
      AttachToInitializedCef();
    }
  }

  ~AegisCefHost() override {
    Shutdown();
    AegisRemoveRuntimeSession(runtime_session_paths_);
  }

  void WaitForReady() {
    AppendDebugLog("host: wait_for_ready enter");
    std::unique_lock lock(mutex_);
    if (!cv_.wait_for(lock, kStartupTimeout, [this] {
          return startup_complete_ || !startup_error_.empty();
        })) {
      throw std::runtime_error("timed out waiting for CEF startup");
    }
    if (!startup_error_.empty()) {
      throw std::runtime_error(startup_error_);
    }
    AppendDebugLog("host: wait_for_ready complete");
  }

  std::vector<std::uint8_t> InstallRuntime(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "install_runtime");
    try {
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();
      return {};
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> EvalJs(const std::vector<std::uint8_t>& request) override {
    OperationScope scope(this, "eval_js");
    try {
      SetOperationStage("decoding eval request");
      auto payload = RequireDictionary(
          DecodeEnvelope(MessageKind::EvalJs, request), "eval request must be a dictionary");
      SetOperationStage("evaluating javascript");
      const auto result =
          InvokeRenderer(aegis::kOpEvalJs, payload->GetString("script").ToString());

      auto response = CefDictionaryValue::Create();
      auto bytes = CefListValue::Create();
      for (std::size_t index = 0; index < result.size(); ++index) {
        bytes->SetInt(static_cast<int>(index),
                      static_cast<unsigned char>(result[static_cast<std::size_t>(index)]));
      }
      response->SetList("value", bytes);
      return EncodeEnvelope(MessageKind::EvalJs, response);
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> SendBatch(const std::vector<std::uint8_t>& request) override {
    OperationScope scope(this, "send_batch");
    try {
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();
      SetOperationStage("decoding batch request");
      auto payload =
          RequireDictionary(DecodeEnvelope(MessageKind::SendBatch, request),
                            "batch request must be a dictionary");
      SetOperationStage("dispatching batch to renderer");
      const auto body = WriteJson(payload);
      auto response = RequireDictionary(
          ParseJsonValue(InvokeRendererReady(aegis::kOpSendBatch, body),
                         "batch response is not valid json"),
          "batch response must be a dictionary");
      MergeLocalEventsIntoResponse(response);
      return EncodeEnvelope(MessageKind::SendBatch, response);
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> SnapshotDom(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "snapshot_dom");
    try {
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();
      SetOperationStage("capturing DOM snapshot");
      return EncodeJsonEnvelope(MessageKind::SnapshotDom,
                                InvokeRendererReady(aegis::kOpSnapshotDom, "{}"));
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> InjectSession(const std::vector<std::uint8_t>& request) override {
    OperationScope scope(this, "inject_session");
    try {
      SetOperationStage("decoding session request");
      auto payload = RequireDictionary(
          DecodeEnvelope(MessageKind::InjectSession, request),
          "session request must be a dictionary");

      SetOperationStage("replacing network overrides");
      ReplaceNetworkOverrides(payload);
      SetOperationStage("replacing cookies");
      ReplaceCookies(payload);
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();
      SetOperationStage("injecting storage");
      InvokeRendererReady(aegis::kOpInjectStorage, WriteJson(payload));
      return {};
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> SnapshotSession(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "snapshot_session");
    try {
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();
      SetOperationStage("capturing document cookies");
      CaptureDocumentCookiesFromActivePage();

      SetOperationStage("capturing storage snapshot");
      auto storage = RequireDictionary(
          ParseJsonValue(InvokeRendererReady(aegis::kOpSnapshotStorage, "{}"),
                         "storage snapshot is not valid json"),
          "storage snapshot must be a dictionary");
      storage->SetList("cookies", SnapshotCookies());
      storage->SetList("network_overrides", SnapshotNetworkOverrides());
      return EncodeEnvelope(MessageKind::SnapshotSession, storage);
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> DrainEvents(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "drain_events");
    try {
      SetOperationStage("ensuring runtime is installed");
      EnsureRuntimeInstalled();

      SetOperationStage("draining renderer events");
      const auto renderer_response = InvokeRendererReady(aegis::kOpDrainEvents, "{}");
      AppendDebugLog("host: drain_events renderer_response bytes=" +
                     std::to_string(renderer_response.size()));
      auto response = RequireDictionary(
          ParseJsonValue(renderer_response,
                         "drain events response is not valid json"),
          "drain events response must be a dictionary");
      AppendDebugLog("host: drain_events parsed_response");
      auto existing_events =
          response->HasKey("events") ? response->GetList("events") : CefListValue::Create();
      AppendDebugLog("host: drain_events existing_events=" +
                     std::to_string(existing_events ? existing_events->GetSize() : 0));
      auto local_events = DrainLocalEvents();
      AppendDebugLog("host: drain_events local_events=" + std::to_string(local_events.size()));
      for (const auto& json : local_events) {
        AppendDebugLog("host: drain_events merge_local_event bytes=" +
                       std::to_string(json.size()));
      }
      MergeEventsIntoResponse(response, std::move(local_events));
      AppendDebugLog("host: drain_events encode_response");
      auto encoded = EncodeEnvelope(MessageKind::DrainEvents, response);
      AppendDebugLog("host: drain_events encoded");
      return encoded;
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> Navigate(const std::vector<std::uint8_t>& request) override {
    OperationScope scope(this, "navigate");
    try {
      SetOperationStage("decoding navigate request");
      auto payload = RequireDictionary(
          DecodeEnvelope(MessageKind::Navigate, request), "navigate request must be a dictionary");
      const auto target_url = payload->GetString("url").ToString();
      SetOperationStage("starting browser navigation");
      NavigateTo(target_url);
      SetOperationStage("waiting for runtime installation after navigation");
      EnsureRuntimeInstalled();

      auto response = CefDictionaryValue::Create();
      response->SetString("url", CurrentUrl());

      auto events = CefListValue::Create();
      events->SetValue(0, NavigationEvent(CurrentUrl()));
      response->SetList("events", events);
      MergeLocalEventsIntoResponse(response);
      AppendDebugLog("host: navigate encode_response");
      auto encoded = EncodeEnvelope(MessageKind::Navigate, response);
      AppendDebugLog("host: navigate encoded");
      return encoded;
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> Pump(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    AegisPumpBrowserHostWindow();
    if (!options_.headless && AegisBrowserHostWindowCloseRequested()) {
      throw std::runtime_error("browser window closed by user");
    }
    CefDoMessageLoopWork();
    return {};
  }

  std::vector<std::uint8_t> SnapshotChromeState(
      const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    std::string json = "{\"title\":\"";
    json += EscapeJsonString(current_title_);
    json += "\",\"url\":\"";
    json += EscapeJsonString(current_url_);
    json += "\",\"can_go_back\":";
    json += can_go_back_ ? "true" : "false";
    json += ",\"can_go_forward\":";
    json += can_go_forward_ ? "true" : "false";
    json += ",\"is_loading\":";
    json += is_loading_ ? "true" : "false";
    json += "}";
    return std::vector<std::uint8_t>(json.begin(), json.end());
  }

  std::vector<std::uint8_t> GoBack(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    if (browser_.get() && browser_->CanGoBack()) {
      browser_->GoBack();
    }
    return {};
  }

  std::vector<std::uint8_t> GoForward(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    if (browser_.get() && browser_->CanGoForward()) {
      browser_->GoForward();
    }
    return {};
  }

  std::vector<std::uint8_t> ReloadPage(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    if (browser_.get()) {
      browser_->Reload();
    }
    return {};
  }

  std::vector<std::uint8_t> StopLoad(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    if (browser_.get()) {
      browser_->StopLoad();
    }
    return {};
  }

  void OnPrimaryBrowserCreated(CefRefPtr<CefBrowser> browser) override {
    AppendDebugLog("host: on_browser_created");
    {
      std::lock_guard lock(mutex_);
      browser_ = browser;
      request_context_ = browser->GetHost()->GetRequestContext();
      page_ready_ = false;
      renderer_ready_ = false;
      runtime_installed_ = false;
      if (auto frame = browser->GetMainFrame(); frame.get()) {
        current_url_ = frame->GetURL().ToString();
      }
      cv_.notify_all();
    }
    if (!options_.headless && browser && AegisUseExternalBrowserHostWindow()) {
      AegisSetBrowserHostAddress(browser->GetMainFrame()->GetURL().ToString());
      AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                         browser->IsLoading());
      AegisAttachBrowserToHostWindow(browser);
      AegisShowBrowserHostWindow();
    }
  }

  void OnBeforeBrowse(CefRefPtr<CefBrowser>,
                      CefRefPtr<CefFrame>,
                      CefRefPtr<CefRequest>) override {
    AppendDebugLog("host: on_before_browse");
    std::lock_guard lock(mutex_);
    page_ready_ = false;
    renderer_ready_ = false;
    runtime_installed_ = false;
  }

  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser, bool is_loading) override {
    AppendDebugLog(std::string("host: on_loading_state_change loading=") +
                   (is_loading ? "true" : "false"));
    {
      std::lock_guard lock(mutex_);
      if (!browser_.get() || !browser->IsSame(browser_)) {
        return;
      }
      is_loading_ = is_loading;
      can_go_back_ = browser->CanGoBack();
      can_go_forward_ = browser->CanGoForward();
      page_ready_ = !is_loading;
      if (!is_loading) {
        current_url_ = browser->GetMainFrame()->GetURL().ToString();
      }
      cv_.notify_all();
    }
    if (!options_.headless && browser) {
      AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                         is_loading);
    }
  }

  void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                 CefRefPtr<CefFrame> frame,
                 int http_status_code) override {
    AppendDebugLog(std::string("host: on_load_end status=") +
                   std::to_string(http_status_code));
    if (!frame.get() || !frame->IsMain()) {
      return;
    }
    std::lock_guard lock(mutex_);
    if (!browser_.get() || !browser->IsSame(browser_)) {
      return;
    }
    current_url_ = frame->GetURL().ToString();
  }

  void OnAddressChange(CefRefPtr<CefBrowser>,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override {
    if (frame.get() && frame->IsMain()) {
      std::lock_guard lock(mutex_);
      current_url_ = url.ToString();
    }
    if (options_.headless || !frame.get() || !frame->IsMain()) {
      return;
    }
    AegisSetBrowserHostAddress(url.ToString());
  }

  void OnTitleChange(CefRefPtr<CefBrowser>,
                     const CefString& title) override {
    {
      std::lock_guard lock(mutex_);
      current_title_ = title.ToString();
    }
    if (options_.headless) {
      return;
    }
    AegisSetBrowserHostTitle(title.ToString());
  }

  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override {
    AppendDebugLog("host: on_before_close");
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() && browser->IsSame(browser_)) {
        browser_ = nullptr;
        request_context_ = nullptr;
        client_ = nullptr;
        page_ready_ = false;
        renderer_ready_ = false;
        runtime_installed_ = false;
        browser_closed_ = true;
        cv_.notify_all();
      }
    }
    if (!options_.headless) {
      AegisCloseBrowserHostWindow();
    }
  }

  cef_return_value_t OnBeforeResourceLoad(CefRefPtr<CefBrowser>,
                                          CefRefPtr<CefFrame> frame,
                                          CefRefPtr<CefRequest> request) override {
    std::lock_guard lock(mutex_);
    RequestTelemetryState telemetry;
    telemetry.method = request->GetMethod().ToString();
    telemetry.url = request->GetURL().ToString();
    telemetry.is_main_frame = frame.get() && frame->IsMain();
    telemetry.started_at = std::chrono::steady_clock::now();
    request_telemetry_[request->GetIdentifier()] = std::move(telemetry);
    if (network_overrides_.empty()) {
      return RV_CONTINUE;
    }

    CefRequest::HeaderMap headers;
    request->GetHeaderMap(headers);
    for (const auto& [header, value] : network_overrides_) {
      auto range = headers.equal_range(header);
      headers.erase(range.first, range.second);
      headers.emplace(header, value);
    }
    request->SetHeaderMap(headers);
    return RV_CONTINUE;
  }

  void OnResourceLoadComplete(CefRefPtr<CefBrowser>,
                              CefRefPtr<CefFrame> frame,
                              CefRefPtr<CefRequest> request,
                              CefRefPtr<CefResponse> response,
                              cef_urlrequest_status_t status,
                              int64_t received_content_length) override {
    CaptureResponseCookies(request, response);
    PushLocalEvent(NetworkEvent(request, response, status, std::nullopt, std::nullopt,
                                std::nullopt, std::nullopt,
                                frame.get() && frame->IsMain(),
                                received_content_length));
  }

  void OnResourceRedirect(CefRefPtr<CefBrowser>,
                          CefRefPtr<CefFrame>,
                          CefRefPtr<CefRequest> request,
                          CefRefPtr<CefResponse> response,
                          const CefString& new_url) override {
    PushLocalEvent(NetworkEvent(request, response, UR_IO_PENDING, new_url.ToString(),
                                std::nullopt, std::nullopt));
  }

  void OnLoadError(CefRefPtr<CefBrowser>,
                   CefRefPtr<CefFrame> frame,
                   CefLoadHandler::ErrorCode error_code,
                   const CefString& error_text,
                   const CefString& failed_url) override {
    PushLocalEvent(NetworkEvent(nullptr, nullptr, UR_FAILED, std::nullopt,
                                failed_url.ToString(), static_cast<int>(error_code),
                                error_text.ToString(),
                                frame.get() && frame->IsMain()));
  }

  bool HandleBrowserProcessMessage(CefRefPtr<CefBrowser>,
                                   CefRefPtr<CefFrame>,
                                   CefProcessId source_process,
                                   CefRefPtr<CefProcessMessage> message) {
    if (source_process != PID_RENDERER || !message.get() ||
        (message->GetName() != aegis::kAegisResponseMessage &&
         message->GetName() != aegis::kAegisLifecycleMessage)) {
      return false;
    }

    if (message->GetName() == aegis::kAegisLifecycleMessage) {
      auto args = message->GetArgumentList();
      if (args->GetString(0).ToString() == aegis::kLifecycleContextReady) {
        AppendDebugLog("host: lifecycle context_ready");
        std::lock_guard lock(mutex_);
        renderer_ready_ = true;
        runtime_installed_ = true;
        const auto url = args->GetString(1).ToString();
        if (!url.empty()) {
          current_url_ = url;
        }
        cv_.notify_all();
        return true;
      }
      return false;
    }

    auto args = message->GetArgumentList();
    AppendDebugLog("host: renderer response received");
    CompleteRendererRequest(args->GetInt(0), args->GetBool(1),
                            args->GetString(2).ToString());
    return true;
  }

 private:
  void BeginOperation(const std::string& name) {
    current_operation_name_ = name;
    current_operation_stage_ = "starting";
    current_operation_started_at_ = std::chrono::steady_clock::now();
  }

  void EndOperation() {
    current_operation_name_.clear();
    current_operation_stage_.clear();
  }

  void SetOperationStage(const std::string& stage) { current_operation_stage_ = stage; }

  std::string WrapOperationError(const std::string& message) const {
    if (current_operation_name_.empty() || IsStructuredOperationError(message)) {
      return message;
    }
    const auto elapsed_ms = static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now() - current_operation_started_at_)
            .count());
    const bool timed_out = message.find("timed out") != std::string::npos;
    const bool restart_recommended =
        timed_out || message.find("browser window closed by user") != std::string::npos ||
        message.find("browser is not available") != std::string::npos;
    return std::string("{") + "\"kind\":\"operation_error\"," + "\"operation\":\"" +
           EscapeJsonString(current_operation_name_) + "\"," + "\"stage\":\"" +
           EscapeJsonString(current_operation_stage_.empty() ? "unknown"
                                                             : current_operation_stage_) +
           "\"," + "\"message\":\"" + EscapeJsonString(message) + "\"," +
           "\"elapsed_ms\":" + std::to_string(elapsed_ms) + "," + "\"timed_out\":" +
           (timed_out ? "true" : "false") + "," + "\"restart_recommended\":" +
           (restart_recommended ? "true" : "false") + "}";
  }

  void RequireOwnerThread() const {
    if (std::this_thread::get_id() != owner_thread_id_) {
      throw std::runtime_error("aegis CEF host methods must run on the owner thread");
    }
  }

  template <typename Predicate>
  void PumpUntil(Predicate predicate,
                 std::chrono::steady_clock::time_point deadline,
                 const char* timeout_message) {
    for (;;) {
      RequireOwnerThread();

      {
        std::lock_guard lock(mutex_);
        if (!startup_error_.empty()) {
          throw std::runtime_error(startup_error_);
        }
        if (predicate()) {
          return;
        }
      }

      if (std::chrono::steady_clock::now() >= deadline) {
        throw std::runtime_error(timeout_message);
      }

      AegisPumpBrowserHostWindow();
      CefDoMessageLoopWork();
      std::this_thread::sleep_for(kPumpInterval);
    }
  }

  std::vector<std::uint8_t> EncodeJsonEnvelope(MessageKind kind, const std::string& json) {
    return EncodeEnvelope(kind, ParseJsonValue(json, "renderer response is not valid json"));
  }

  CefRefPtr<CefValue> NavigationEvent(const std::string& url) {
    auto event = CefDictionaryValue::Create();
    auto body = CefDictionaryValue::Create();
    body->SetString("type", "navigation");
    body->SetString("url", url);
    event->SetDictionary("event", body);

    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(event);
    return wrapped;
  }

  CefRefPtr<CefDictionaryValue> HeaderDictionary(const CefResponse::HeaderMap& headers) {
    auto dict = CefDictionaryValue::Create();
    for (const auto& [header, value] : headers) {
      dict->SetString(header, value);
    }
    return dict;
  }

  CefRefPtr<CefValue> NetworkEvent(
      CefRefPtr<CefRequest> request,
      CefRefPtr<CefResponse> response,
      cef_urlrequest_status_t status,
      std::optional<std::string> redirect_url = std::nullopt,
      std::optional<std::string> failed_url = std::nullopt,
      std::optional<int> error_code = std::nullopt,
      std::optional<std::string> error_text = std::nullopt,
      std::optional<bool> is_main_frame = std::nullopt,
      std::optional<int64_t> received_content_length = std::nullopt) {
    auto event = CefDictionaryValue::Create();
    auto body = CefDictionaryValue::Create();
    body->SetString("type", "network");
    std::optional<std::uint64_t> duration_ms;
    if (request.get()) {
      const auto request_id = request->GetIdentifier();
      body->SetString("request_id", std::to_string(request_id));
      body->SetString("url", request->GetURL().ToString());
      body->SetString("method", request->GetMethod().ToString());
      {
        std::lock_guard lock(mutex_);
        auto it = request_telemetry_.find(request_id);
        if (it != request_telemetry_.end()) {
          duration_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                            std::chrono::steady_clock::now() - it->second.started_at)
                            .count();
          if (!body->HasKey("method") && !it->second.method.empty()) {
            body->SetString("method", it->second.method);
          }
          if (!body->HasKey("url") && !it->second.url.empty()) {
            body->SetString("url", it->second.url);
          }
          if (!is_main_frame.has_value()) {
            is_main_frame = it->second.is_main_frame;
          }
          request_telemetry_.erase(it);
        }
      }
    }
    if (response.get()) {
      body->SetInt("status_code", response->GetStatus());
      body->SetString("status_text", response->GetStatusText().ToString());
      body->SetString("mime_type", response->GetMimeType().ToString());
      CefResponse::HeaderMap headers;
      response->GetHeaderMap(headers);
      auto it = headers.find("Content-Length");
      if (it != headers.end()) {
        const auto content_length = it->second.ToString();
        char* end = nullptr;
        errno = 0;
        const auto parsed = std::strtoll(content_length.c_str(), &end, 10);
        if (end != content_length.c_str() && errno == 0 &&
            parsed <= std::numeric_limits<int>::max() &&
            parsed >= std::numeric_limits<int>::min()) {
          body->SetInt("content_length_bytes", static_cast<int>(parsed));
        }
      }
      body->SetDictionary("response_headers", HeaderDictionary(headers));
    }
    if (redirect_url.has_value() && !redirect_url->empty()) {
      body->SetString("redirect_url", *redirect_url);
    }
    if (failed_url.has_value() && !failed_url->empty()) {
      body->SetString("url", *failed_url);
    }
    if (error_code.has_value()) {
      body->SetInt("error_code", *error_code);
    }
    if (error_text.has_value() && !error_text->empty()) {
      body->SetString("error_text", *error_text);
    }
    if (is_main_frame.has_value()) {
      body->SetBool("is_main_frame", *is_main_frame);
    }
    if (received_content_length.has_value() && *received_content_length >= 0 &&
        *received_content_length <= std::numeric_limits<int>::max()) {
      body->SetInt("received_content_length_bytes",
                   static_cast<int>(*received_content_length));
    }
    if (duration_ms.has_value() && *duration_ms <= std::numeric_limits<int>::max()) {
      body->SetInt("duration_ms", static_cast<int>(*duration_ms));
    }
    body->SetString("request_status", UrlRequestStatusToString(status));
    event->SetDictionary("event", body);

    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(event);
    return wrapped;
  }

  void MergeEventsIntoResponse(CefRefPtr<CefDictionaryValue> response,
                               std::vector<std::string> local_events) {
    auto existing_events =
        response->HasKey("events") ? response->GetList("events") : CefListValue::Create();
    auto events = CefListValue::Create();
    if (existing_events.get()) {
      for (size_t index = 0; index < existing_events->GetSize(); ++index) {
        auto value = existing_events->GetValue(static_cast<int>(index));
        if (value.get()) {
          events->SetValue(static_cast<int>(index), value->Copy());
        }
      }
    }

    auto index = static_cast<int>(events->GetSize());
    for (auto& json : local_events) {
      events->SetValue(index++, ParseJsonValue(json, "local event is not valid json"));
    }
    response->SetList("events", events);
  }

  void MergeLocalEventsIntoResponse(CefRefPtr<CefDictionaryValue> response) {
    MergeEventsIntoResponse(response, DrainLocalEvents());
  }

  void PushLocalEvent(CefRefPtr<CefValue> event) {
    std::lock_guard lock(mutex_);
    local_events_.push_back(WriteJson(event));
  }

  std::vector<std::string> DrainLocalEvents() {
    std::lock_guard lock(mutex_);
    auto events = std::move(local_events_);
    local_events_.clear();
    return events;
  }

  void Start() {
    try {
      AppendDebugLog("host: start");
      RequireOwnerThread();

      AppendDebugLog("host: cef runtime load begin");
      std::string load_error;
      if (!AegisPlatformLoadCefRuntime(paths_.cef_library, &load_error)) {
        throw std::runtime_error(load_error.empty()
                                     ? "failed to load Chromium Embedded Framework runtime"
                                     : load_error);
      }
      AppendDebugLog("host: cef runtime load complete");

      CefMainArgs main_args;
      AegisCefBootstrapOptions bootstrap_options;
      bootstrap_options.headless = options_.headless;
      bootstrap_options.external_message_pump = true;
      bootstrap_options.initialize_browser_host_application = !options_.headless;
      bootstrap_options.browser_subprocess_path = paths_.helper_executable.string();
      bootstrap_options.framework_dir_path = paths_.framework_dir.string();
      bootstrap_options.main_bundle_path = paths_.main_bundle_path.string();
      bootstrap_options.resources_dir_path = paths_.resources_dir.string();
      bootstrap_options.locales_dir_path = paths_.locales_dir.string();
      bootstrap_options.root_cache_path = runtime_session_paths_.instance_dir.string();
      bootstrap_options.cache_path =
          (runtime_session_paths_.instance_dir / "cache").string();
      app_ = new AegisApp(false);
      int subprocess_exit_code = -1;
      std::string initialize_error;
      AppendDebugLog("host: canonical cef bootstrap begin");
      const bool initialized = AegisExecuteProcessAndInitialize(
          main_args, bootstrap_options, app_, &subprocess_exit_code, &initialize_error);
      AppendDebugLog("host: canonical cef bootstrap subprocess_exit_code=" +
                     std::to_string(subprocess_exit_code));
      if (subprocess_exit_code >= 0) {
        throw std::runtime_error("unexpected subprocess execution in embedded host");
      }
      if (!initialized) {
        throw std::runtime_error(
            initialize_error.empty() ? "canonical cef bootstrap failed" : initialize_error);
      }
      cef_initialized_ = true;
      ApplyAegisProductionPreferences(CefPreferenceManager::GetGlobalPreferenceManager());
      AppendDebugLog("host: cef initialized");

      CreateBrowserOnUiThread();
      AppendDebugLog("host: create browser dispatched");

      {
        std::lock_guard lock(mutex_);
        startup_complete_ = true;
        cv_.notify_all();
      }
    } catch (const std::exception& error) {
      AppendDebugLog(std::string("host: startup_error ") + error.what());
      if (cef_initialized_) {
        CefShutdown();
        AegisPlatformUnloadCefRuntime();
        cef_initialized_ = false;
      }
      AegisRemoveRuntimeSession(runtime_session_paths_);
      std::lock_guard lock(mutex_);
      startup_error_ = error.what();
      cv_.notify_all();
    }
  }

  void Shutdown() {
    if (!manage_cef_lifecycle_) {
      return;
    }
    if (!cef_initialized_) {
      return;
    }
    if (std::this_thread::get_id() != owner_thread_id_) {
      return;
    }

    try {
      AppendDebugLog("host: shutdown");
      const auto has_browser = [this]() {
        std::lock_guard lock(mutex_);
        return browser_.get() != nullptr;
      }();
      if (has_browser) {
        CloseBrowserOnUiThread();
        const auto deadline = std::chrono::steady_clock::now() + kShutdownTimeout;
        PumpUntil([this]() { return browser_closed_ || browser_.get() == nullptr; }, deadline,
                  "timed out waiting for browser shutdown");
      }
      CefShutdown();
      AegisPlatformUnloadCefRuntime();
      cef_initialized_ = false;
    } catch (...) {
      cef_initialized_ = false;
    }
  }

  void AttachToInitializedCef() {
    RequireOwnerThread();
    AppendDebugLog("host: attach_to_initialized_cef");
    cef_initialized_ = true;
    CreateBrowserOnUiThread();
    {
      std::lock_guard lock(mutex_);
      startup_complete_ = true;
      cv_.notify_all();
    }
  }

  void CreateBrowserOnUiThread() {
    auto task = [this]() {
      CEF_REQUIRE_UI_THREAD();
      AppendDebugLog("host: create_browser_on_ui_thread");

      CefRequestContextSettings request_context_settings;
      request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
      if (!request_context_.get()) {
        throw std::runtime_error("failed to create request context");
      }
      ApplyAegisProductionPreferences(request_context_);

      CefBrowserSettings settings;
      settings.windowless_frame_rate = 30;

      client_ = new AegisHostClient(options_.headless,
                                    static_cast<::AegisClientDelegate*>(this), this);
      const auto initial_url = options_.start_url.empty() ? std::string(kBootstrapUrl)
                                                          : options_.start_url;

      if (options_.headless) {
        CefWindowInfo window_info;
        window_info.SetAsWindowless(kNullWindowHandle);
        window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
        if (!CefBrowserHost::CreateBrowser(window_info, client_, initial_url, settings, nullptr,
                                           request_context_)) {
          throw std::runtime_error("failed to create headless browser");
        }
        AppendDebugLog("host: create headless browser requested");
        return;
      }
      CefWindowInfo window_info;
      if (AegisUseExternalBrowserHostWindow()) {
        window_info.SetAsChild(AegisCreateBrowserHostView("Aegis", 1280, 800),
                               CefRect(0, 0, 1280, 800));
      } else {
        AegisPlatformConfigureTopLevelWindow(&window_info, "Aegis", 1280, 800);
      }
      window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
      if (!CefBrowserHost::CreateBrowser(window_info, client_, initial_url, settings, nullptr,
                                         request_context_)) {
        throw std::runtime_error("failed to create headful browser");
      }
      AppendDebugLog("host: create headful browser requested");
    };

    if (CefCurrentlyOn(TID_UI)) {
      task();
      return;
    }
    RunOnUiThreadSync(task);
  }

  void CloseBrowserOnUiThread() {
    auto task = [this]() {
      CEF_REQUIRE_UI_THREAD();
      if (browser_.get()) {
        browser_->GetHost()->CloseBrowser(true);
      }
    };
    if (CefCurrentlyOn(TID_UI)) {
      task();
      return;
    }
    RunOnUiThreadSync(task);
  }

  void RunOnUiThreadSync(const std::function<void()>& work) {
    RequireOwnerThread();
    if (CefCurrentlyOn(TID_UI)) {
      work();
      return;
    }
    auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    auto event = CefWaitableEvent::CreateWaitableEvent(true, false);
    auto error = std::make_shared<std::exception_ptr>();
    CefPostTask(TID_UI, new UiClosureTask(work, error, event));
    PumpUntil([&event]() { return event->IsSignaled(); }, deadline,
              "timed out waiting for UI task");
    if (*error) {
      std::rethrow_exception(*error);
    }
  }

  void EnsurePageReady() {
    RequireOwnerThread();
    AppendDebugLog("host: ensure_page_ready enter");
    const auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    PumpUntil([this]() { return startup_complete_ || !startup_error_.empty(); }, deadline,
              "timed out waiting for CEF startup");
    PumpUntil(
        [this]() {
          return browser_.get() != nullptr && renderer_ready_ && page_ready_;
        },
        deadline, "timed out waiting for browser page readiness");
    AppendDebugLog("host: ensure_page_ready complete");
  }

  void WaitForReadyLocked(std::unique_lock<std::mutex>& lock) {
    if (!startup_complete_ && startup_error_.empty()) {
      cv_.wait_for(lock, kStartupTimeout, [this] {
        return startup_complete_ || !startup_error_.empty();
      });
    }
    if (!startup_error_.empty()) {
      throw std::runtime_error(startup_error_);
    }
  }

  std::string CurrentUrl() {
    RequireOwnerThread();
    SetOperationStage("reading current browser url");
    AegisPumpBrowserHostWindow();
    CefDoMessageLoopWork();
    std::lock_guard lock(mutex_);
    return current_url_;
  }

  void NavigateTo(const std::string& url) {
    SetOperationStage("preparing browser navigation");
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() != nullptr && page_ready_ && current_url_ == url) {
        AppendDebugLog("host: navigate_to skipped_same_url");
        return;
      }
    }
    {
      std::lock_guard lock(mutex_);
      page_ready_ = false;
      renderer_ready_ = false;
    }
    SetOperationStage("dispatching LoadURL on UI thread");
    RunOnUiThreadSync([this, url]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      browser_->GetMainFrame()->LoadURL(url);
    });
  }

  void EnsureRuntimeInstalled() {
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() != nullptr && renderer_ready_ && page_ready_ && runtime_installed_) {
        return;
      }
    }
    SetOperationStage("waiting for ready browser page");
    EnsurePageReady();
    std::lock_guard lock(mutex_);
    runtime_installed_ = true;
  }

  std::string InvokeRendererReady(const std::string& operation, const std::string& body) {
    AppendDebugLog("host: invoke_renderer " + operation);
    SetOperationStage(std::string("dispatching renderer operation: ") + operation);

    const int request_id = [this] {
      std::lock_guard lock(mutex_);
      return next_request_id_++;
    }();

    RunOnUiThreadSync([this, request_id, operation, body]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      auto frame = browser_->GetMainFrame();
      if (!frame.get()) {
        throw std::runtime_error("main frame is not available");
      }

      auto message = CefProcessMessage::Create(aegis::kAegisRequestMessage);
      auto args = message->GetArgumentList();
      args->SetInt(0, request_id);
      args->SetString(1, operation);
      args->SetString(2, body);
      frame->SendProcessMessage(PID_RENDERER, message);
    });

    const auto deadline = std::chrono::steady_clock::now() + kRendererTimeout;
    SetOperationStage(std::string("waiting for renderer reply: ") + operation);
    PumpUntil([this, request_id]() {
      return renderer_replies_.contains(request_id) || !startup_error_.empty();
    }, deadline, "timed out waiting for renderer response");

    RendererReply reply;
    {
      std::lock_guard lock(mutex_);
      reply = std::move(renderer_replies_.at(request_id));
      renderer_replies_.erase(request_id);
    }
    if (!reply.ok) {
      throw std::runtime_error(reply.body);
    }
    AppendDebugLog("host: invoke_renderer complete " + operation);
    return reply.body;
  }

  std::string InvokeRenderer(const std::string& operation, const std::string& body) {
    EnsurePageReady();
    return InvokeRendererReady(operation, body);
  }

  void CompleteRendererRequest(int request_id, bool ok, std::string body) {
    std::lock_guard lock(mutex_);
    renderer_replies_[request_id] = RendererReply{
        .ok = ok,
        .body = std::move(body),
    };
    cv_.notify_all();
  }

  void UpsertManagedCookie(ManagedCookie cookie) {
    std::lock_guard lock(mutex_);
    auto matches = [&](const ManagedCookie& existing) {
      return existing.name == cookie.name && existing.domain == cookie.domain &&
             existing.path == cookie.path;
    };

    const bool remove_cookie =
        cookie.value.empty() ||
        (cookie.expires_unix.has_value() && *cookie.expires_unix == 0);
    auto existing = std::find_if(cookie_jar_.begin(), cookie_jar_.end(), matches);
    if (remove_cookie) {
      if (existing != cookie_jar_.end()) {
        cookie_jar_.erase(existing);
      }
      return;
    }
    if (existing != cookie_jar_.end()) {
      *existing = std::move(cookie);
      return;
    }
    cookie_jar_.push_back(std::move(cookie));
  }

  void ReplaceManagedCookies(CefRefPtr<CefListValue> cookies) {
    std::vector<ManagedCookie> jar;
    if (cookies.get()) {
      for (std::size_t index = 0; index < cookies->GetSize(); ++index) {
        auto cookie_value = RequireDictionary(
            cookies->GetValue(static_cast<int>(index)), "cookie must be a dictionary");
        ManagedCookie cookie{
            .name = cookie_value->GetString("name").ToString(),
            .value = cookie_value->GetString("value").ToString(),
            .domain = cookie_value->GetString("domain").ToString(),
            .path = cookie_value->HasKey("path")
                        ? cookie_value->GetString("path").ToString()
                        : std::string("/"),
            .expires_unix = cookie_value->HasKey("expires_unix")
                                ? std::optional<std::uint64_t>(static_cast<std::uint64_t>(
                                      cookie_value->GetDouble("expires_unix")))
                                : std::nullopt,
            .secure = cookie_value->HasKey("secure") && cookie_value->GetBool("secure"),
            .http_only =
                cookie_value->HasKey("http_only") && cookie_value->GetBool("http_only"),
        };
        jar.push_back(std::move(cookie));
      }
    }
    std::lock_guard lock(mutex_);
    cookie_jar_ = std::move(jar);
  }

  std::optional<ManagedCookie> ParseSetCookieHeader(const std::string& url,
                                                    const std::string& header_value) {
    auto host = UrlHost(url);
    if (!host.has_value() || host->empty()) {
      return std::nullopt;
    }

    ManagedCookie cookie{
        .domain = *host,
        .path = "/",
        .secure = UrlScheme(url).value_or("") == "https",
    };
    bool saw_name_value = false;

    std::size_t start = 0;
    while (start <= header_value.size()) {
      const auto delimiter = header_value.find(';', start);
      auto token = TrimAscii(header_value.substr(start, delimiter - start));
      start = delimiter == std::string::npos ? header_value.size() + 1 : delimiter + 1;
      if (token.empty()) {
        continue;
      }
      const auto equals = token.find('=');
      if (!saw_name_value) {
        if (equals == std::string::npos) {
          return std::nullopt;
        }
        cookie.name = TrimAscii(token.substr(0, equals));
        cookie.value = token.substr(equals + 1);
        saw_name_value = !cookie.name.empty();
        continue;
      }

      const auto key = TrimAscii(token.substr(0, equals));
      const auto value =
          equals == std::string::npos ? std::string() : TrimAscii(token.substr(equals + 1));
      if (CaseEqualAscii(key, "domain") && !value.empty()) {
        cookie.domain = value.front() == '.' ? value.substr(1) : value;
      } else if (CaseEqualAscii(key, "path") && !value.empty()) {
        cookie.path = value;
      } else if (CaseEqualAscii(key, "secure")) {
        cookie.secure = true;
      } else if (CaseEqualAscii(key, "httponly")) {
        cookie.http_only = true;
      } else if (CaseEqualAscii(key, "max-age")) {
        try {
          if (std::stoll(value) <= 0) {
            cookie.expires_unix = 0;
          }
        } catch (...) {
        }
      }
    }

    if (!saw_name_value) {
      return std::nullopt;
    }
    return cookie;
  }

  void CaptureResponseCookies(CefRefPtr<CefRequest> request, CefRefPtr<CefResponse> response) {
    if (!request.get() || !response.get()) {
      return;
    }
    CefResponse::HeaderMap headers;
    response->GetHeaderMap(headers);
    for (const auto& [header, value] : headers) {
      if (!CaseEqualAscii(header.ToString(), "set-cookie")) {
        continue;
      }
      auto cookie = ParseSetCookieHeader(request->GetURL().ToString(), value.ToString());
      if (cookie.has_value()) {
        UpsertManagedCookie(std::move(*cookie));
      }
    }
  }

  void CaptureDocumentCookiesFromActivePage() {
    SetOperationStage("capturing document.cookie from active page");
    std::string url;
    {
      std::lock_guard lock(mutex_);
      url = current_url_;
    }
    auto host = UrlHost(url);
    if (!host.has_value() || host->empty()) {
      return;
    }

    const auto cookie_string = InvokeRendererReady(aegis::kOpEvalJs, "document.cookie");
    std::size_t start = 0;
    while (start <= cookie_string.size()) {
      const auto delimiter = cookie_string.find(';', start);
      auto token = TrimAscii(cookie_string.substr(start, delimiter - start));
      start = delimiter == std::string::npos ? cookie_string.size() + 1 : delimiter + 1;
      if (token.empty()) {
        continue;
      }
      const auto equals = token.find('=');
      if (equals == std::string::npos) {
        continue;
      }
      UpsertManagedCookie(ManagedCookie{
          .name = TrimAscii(token.substr(0, equals)),
          .value = token.substr(equals + 1),
          .domain = *host,
          .path = UrlPath(url).value_or("/"),
          .expires_unix = std::nullopt,
          .secure = UrlScheme(url).value_or("") == "https",
          .http_only = false,
      });
    }
  }

  void ReplaceNetworkOverrides(CefRefPtr<CefDictionaryValue> session) {
    std::vector<std::pair<std::string, std::string>> overrides;
    if (session->HasKey("network_overrides")) {
      auto list = session->GetList("network_overrides");
      for (std::size_t index = 0; index < list->GetSize(); ++index) {
        auto override_value =
            RequireDictionary(list->GetValue(static_cast<int>(index)),
                              "network override must be a dictionary");
        overrides.emplace_back(override_value->GetString("header").ToString(),
                               override_value->GetString("value").ToString());
      }
    }

    std::lock_guard lock(mutex_);
    network_overrides_ = std::move(overrides);
  }

  void ReplaceCookies(CefRefPtr<CefDictionaryValue> session) {
    SetOperationStage("replacing browser cookies");
    auto manager = request_context_->GetCookieManager(nullptr);

    auto clear_event = CefWaitableEvent::CreateWaitableEvent(true, false);
    manager->DeleteCookies("", "", new DeleteCookieSignal(clear_event));
    clear_event->Wait();

    if (!session->HasKey("cookies")) {
      ReplaceManagedCookies(CefListValue::Create());
      return;
    }

    auto cookies = session->GetList("cookies");
    ReplaceManagedCookies(cookies);
    for (std::size_t index = 0; index < cookies->GetSize(); ++index) {
      auto cookie_value = RequireDictionary(
          cookies->GetValue(static_cast<int>(index)), "cookie must be a dictionary");

      CefCookie cookie{};
      CefString(&cookie.name) = cookie_value->GetString("name");
      CefString(&cookie.value) = cookie_value->GetString("value");
      CefString(&cookie.domain) = cookie_value->GetString("domain");
      CefString(&cookie.path) =
          cookie_value->HasKey("path") ? cookie_value->GetString("path") : CefString("/");
      cookie.secure = cookie_value->HasKey("secure") && cookie_value->GetBool("secure");
      cookie.httponly =
          cookie_value->HasKey("http_only") && cookie_value->GetBool("http_only");

      if (cookie_value->HasKey("expires_unix")) {
        cookie.has_expires = true;
        cef_time_t expires{};
        cef_time_from_timet(
            static_cast<time_t>(cookie_value->GetDouble("expires_unix")), &expires);
        cef_time_to_basetime(&expires, &cookie.expires);
      }

      auto set_event = CefWaitableEvent::CreateWaitableEvent(true, false);
      manager->SetCookie(CookieUrl(cookie_value), cookie, new SetCookieSignal(set_event));
      set_event->Wait();
    }

    auto flush_event = CefWaitableEvent::CreateWaitableEvent(true, false);
    manager->FlushStore(new CompletionSignal(flush_event));
    flush_event->Wait();
  }

  CefRefPtr<CefListValue> SnapshotCookies() {
    auto list = CefListValue::Create();
    std::lock_guard lock(mutex_);
    for (std::size_t index = 0; index < cookie_jar_.size(); ++index) {
      const auto& cookie = cookie_jar_[index];
      auto entry = CefDictionaryValue::Create();
      entry->SetString("name", cookie.name);
      entry->SetString("value", cookie.value);
      entry->SetString("domain", cookie.domain);
      entry->SetString("path", cookie.path);
      entry->SetBool("secure", cookie.secure);
      entry->SetBool("http_only", cookie.http_only);
      if (cookie.expires_unix.has_value()) {
        entry->SetDouble("expires_unix", static_cast<double>(*cookie.expires_unix));
      }
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  CefRefPtr<CefListValue> SnapshotNetworkOverrides() {
    std::lock_guard lock(mutex_);
    auto list = CefListValue::Create();
    for (std::size_t index = 0; index < network_overrides_.size(); ++index) {
      auto entry = CefDictionaryValue::Create();
      entry->SetString("header", network_overrides_[index].first);
      entry->SetString("value", network_overrides_[index].second);
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  const BrowserOptions options_;
  const HostPaths paths_;
  const AegisRuntimeSessionPaths runtime_session_paths_;
  const std::thread::id owner_thread_id_;
  const bool manage_cef_lifecycle_;

  CefRefPtr<AegisApp> app_;
  CefRefPtr<AegisClient> client_;

  mutable std::mutex mutex_;
  std::condition_variable cv_;
  bool cef_initialized_ = false;
  bool startup_complete_ = false;
  bool page_ready_ = false;
  bool renderer_ready_ = false;
  bool runtime_installed_ = false;
  bool browser_closed_ = false;
  std::string startup_error_;
  std::string current_url_ = "about:blank";
  std::string current_title_ = "Aegis";
  bool can_go_back_ = false;
  bool can_go_forward_ = false;
  bool is_loading_ = false;
  int next_request_id_ = 1;
  std::vector<std::pair<std::string, std::string>> network_overrides_;
  std::vector<ManagedCookie> cookie_jar_;
  std::vector<std::string> local_events_;
  std::map<std::uint64_t, RequestTelemetryState> request_telemetry_;
  std::map<int, RendererReply> renderer_replies_;
  std::string current_operation_name_;
  std::string current_operation_stage_;
  std::chrono::steady_clock::time_point current_operation_started_at_ =
      std::chrono::steady_clock::now();

  CefRefPtr<CefBrowser> browser_;
  CefRefPtr<CefRequestContext> request_context_;
};

OperationScope::OperationScope(AegisCefHost* host, std::string name) : host_(host) {
  if (host_ != nullptr) {
    host_->BeginOperation(name);
  }
}

OperationScope::~OperationScope() {
  if (host_ != nullptr) {
    host_->EndOperation();
  }
}

bool AegisHostClient::OnProcessMessageReceived(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefProcessId source_process,
    CefRefPtr<CefProcessMessage> message) {
  return host_ &&
         host_->HandleBrowserProcessMessage(browser, frame, source_process, message);
}

template <typename Method>
AegisHostStatus Dispatch(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output,
    Method method) {
  try {
    if (ctx == nullptr) {
      throw std::runtime_error("cef host context is null");
    }

    auto* host = static_cast<CefHost*>(ctx);
    WriteOutput((host->*method)(CopyInput(input_ptr, input_len)), output);
    return AEGIS_HOST_OK;
  } catch (const std::exception& error) {
    const auto message = std::string(error.what());
    WriteOutput(std::vector<std::uint8_t>(message.begin(), message.end()), output);
    return AEGIS_HOST_ERROR;
  }
}

AegisHostStatus InstallRuntime(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::InstallRuntime);
}

AegisHostStatus EvalJs(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::EvalJs);
}

AegisHostStatus SendBatch(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SendBatch);
}

AegisHostStatus SnapshotDom(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotDom);
}

AegisHostStatus InjectSession(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::InjectSession);
}

AegisHostStatus SnapshotSession(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotSession);
}

AegisHostStatus DrainEvents(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::DrainEvents);
}

AegisHostStatus Navigate(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::Navigate);
}

AegisHostStatus Pump(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::Pump);
}

AegisHostStatus SnapshotChromeState(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotChromeState);
}

AegisHostStatus HostGoBack(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::GoBack);
}

AegisHostStatus HostGoForward(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::GoForward);
}

AegisHostStatus HostReloadPage(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::ReloadPage);
}

AegisHostStatus HostStopLoad(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::StopLoad);
}

void FreeBuffer(AegisHostHandle, AegisHostBuffer buffer) {
  delete[] buffer.ptr;
}

}  // namespace

AegisHostFunctionTable ExportFunctionTable() {
  return AegisHostFunctionTable{
      .install_runtime = InstallRuntime,
      .eval_js = EvalJs,
      .send_batch = SendBatch,
      .snapshot_dom = SnapshotDom,
      .inject_session = InjectSession,
      .snapshot_session = SnapshotSession,
      .drain_events = DrainEvents,
      .navigate = Navigate,
      .pump = Pump,
      .snapshot_chrome_state = SnapshotChromeState,
      .go_back = HostGoBack,
      .go_forward = HostGoForward,
      .reload_page = HostReloadPage,
      .stop_load = HostStopLoad,
      .free_buffer = FreeBuffer,
  };
}

}  // namespace aegis

namespace aegis {

bool RunEmbeddedHostOperation(const std::vector<std::uint8_t>& config,
                              EmbeddedHostOperation operation,
                              const std::vector<std::uint8_t>& request,
                              std::vector<std::uint8_t>* response,
                              std::string* error) {
  try {
    if (response == nullptr) {
      throw std::runtime_error("response buffer is null");
    }
    response->clear();

    AegisCefHost host(ParseBrowserOptions(config), false);
    host.WaitForReady();

    if (operation != EmbeddedHostOperation::InstallRuntime) {
      const auto runtime_path = std::filesystem::current_path() / "assets" / "js" / "aegis_runtime.js";
      if (std::filesystem::exists(runtime_path)) {
        std::ifstream runtime_input(runtime_path, std::ios::binary);
        if (runtime_input.is_open()) {
          const std::string script((std::istreambuf_iterator<char>(runtime_input)),
                                   std::istreambuf_iterator<char>());
          auto runtime_value = CefValue::Create();
          runtime_value->SetString(script);
          host.InstallRuntime(EncodeEnvelope(MessageKind::InstallRuntime, runtime_value));
        }
      }
    }

    switch (operation) {
      case EmbeddedHostOperation::InstallRuntime:
        *response = host.InstallRuntime(request);
        break;
      case EmbeddedHostOperation::EvalJs:
        *response = host.EvalJs(request);
        break;
      case EmbeddedHostOperation::SendBatch:
        *response = host.SendBatch(request);
        break;
      case EmbeddedHostOperation::SnapshotDom:
        *response = host.SnapshotDom(request);
        break;
      case EmbeddedHostOperation::InjectSession:
        *response = host.InjectSession(request);
        break;
      case EmbeddedHostOperation::SnapshotSession:
        *response = host.SnapshotSession(request);
        break;
      case EmbeddedHostOperation::DrainEvents:
        *response = host.DrainEvents(request);
        break;
      case EmbeddedHostOperation::Navigate:
        *response = host.Navigate(request);
        break;
      default:
        throw std::runtime_error("unsupported embedded host operation");
    }

    return true;
  } catch (const std::exception& ex) {
    if (error != nullptr) {
      *error = ex.what();
    }
    return false;
  }
}

}  // namespace aegis

extern "C" AegisHostHandle aegis_create_host(const std::uint8_t* input_ptr, std::size_t input_len) {
  aegis::g_last_host_error.clear();
  try {
    auto host = std::make_unique<aegis::AegisCefHost>(
        aegis::ParseBrowserOptions(aegis::CopyInput(input_ptr, input_len)));
    host->WaitForReady();
    return host.release();
  } catch (const std::exception& ex) {
    aegis::g_last_host_error = ex.what();
    return nullptr;
  } catch (...) {
    aegis::g_last_host_error = "unknown native host startup failure";
    return nullptr;
  }
}

extern "C" const char* aegis_last_error_message(void) {
  return aegis::g_last_host_error.empty() ? nullptr : aegis::g_last_host_error.c_str();
}

extern "C" void aegis_destroy_host(AegisHostHandle handle) {
  delete static_cast<aegis::CefHost*>(handle);
}

extern "C" AegisHostFunctionTable aegis_get_function_table(void) {
  return aegis::ExportFunctionTable();
}
