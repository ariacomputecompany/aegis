#include "aegis_cef_host.hpp"

#include <dlfcn.h>

#include <algorithm>
#include <atomic>
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
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <pthread.h>
#include <sstream>
#include <signal.h>
#include <stdexcept>
#include <string>
#include <thread>
#include <unistd.h>
#include <utility>
#include <vector>

#include "../aegis_app.h"
#include "../aegis_bootstrap.h"
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
#include "include/cef_resource_handler.h"
#include "include/cef_waitable_event.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_window.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"
#include "include/wrapper/cef_library_loader.h"
#include "include/wrapper/cef_stream_resource_handler.h"

namespace aegis {
namespace {

thread_local std::string g_last_host_error;
std::mutex g_shared_host_lifecycle_mutex;
std::size_t g_shared_host_count = 0;

constexpr auto kStartupTimeout = std::chrono::seconds(30);
constexpr auto kRendererTimeout = std::chrono::seconds(30);
constexpr auto kShutdownTimeout = std::chrono::seconds(2);
constexpr auto kPumpInterval = std::chrono::milliseconds(10);
void AppendDebugLog(const std::string& message);

const std::string& BootstrapPageHtml() {
  static const std::string html = [] {
    std::ostringstream builder;
    builder << "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">"
            << "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
            << "<title>Aegis Bootstrap</title>"
            << "<style>"
            << ":root{color-scheme:light;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;}"
            << "body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;"
            << "background:linear-gradient(180deg,#f6f8fb 0%,#e8eef8 100%);color:#162033;}"
            << ".card{max-width:640px;margin:32px;padding:28px 32px;border-radius:20px;"
            << "background:rgba(255,255,255,0.9);box-shadow:0 24px 80px rgba(22,32,51,0.16);}"
            << "h1{margin:0 0 12px;font-size:28px;line-height:1.15;}"
            << "p{margin:0 0 12px;font-size:15px;line-height:1.6;}"
            << "code{padding:2px 6px;border-radius:999px;background:#eef3fb;font-size:13px;}"
            << "</style></head><body><main class=\"card\"><h1>Aegis is ready</h1>"
            << "<p>This browser is attached and waiting for a real navigation target.</p>"
            << "<p>The synthetic bootstrap page lives at <code>"
            << aegis::kBootstrapUrl
            << "</code> so startup stays stable without exposing a raw <code>data:</code> URL.</p>"
            << "<p>Navigate to your app or call <code>/navigate</code> to begin automation.</p>"
            << "</main></body></html>";
    return builder.str();
  }();
  return html;
}

class BootstrapSchemeHandlerFactory : public CefSchemeHandlerFactory {
 public:
  CefRefPtr<CefResourceHandler> Create(CefRefPtr<CefBrowser>,
                                       CefRefPtr<CefFrame>,
                                       const CefString&,
                                       CefRefPtr<CefRequest>) override {
    const auto& html = BootstrapPageHtml();
    return new CefStreamResourceHandler(
        "text/html",
        CefStreamReader::CreateForData(
            const_cast<char*>(html.data()), html.size()));
  }

  IMPLEMENT_REFCOUNTING(BootstrapSchemeHandlerFactory);
};

std::string ThreadLabel() {
  std::ostringstream output;
  output << "thread=" << std::this_thread::get_id()
         << " main=" << (pthread_main_np() != 0 ? "true" : "false");
  if (CefCurrentlyOn(TID_UI)) {
    output << " cef_ui=true";
  }
  return output.str();
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

void ApplyStringPreference(CefRefPtr<CefPreferenceManager> manager,
                           const char* name,
                           const std::string& value) {
  if (!manager.get() || !manager->HasPreference(name) ||
      !manager->CanSetPreference(name)) {
    return;
  }

  auto pref_value = CefValue::Create();
  pref_value->SetString(value);
  CefString error;
  if (!manager->SetPreference(name, pref_value, error)) {
    AppendDebugLog(std::string("host: failed_to_set_preference ") + name + " " +
                   error.ToString());
  }
}

void ApplyAegisProductionPreferences(CefRefPtr<CefPreferenceManager> manager,
                                     const std::string& download_dir) {
  ApplyBooleanPreference(manager, "credentials_enable_service", true);
  ApplyBooleanPreference(manager, "profile.password_manager_enabled", true);
  ApplyBooleanPreference(manager, "profile.password_manager_leak_detection", true);
  ApplyBooleanPreference(manager, "autofill.profile_enabled", true);
  ApplyBooleanPreference(manager, "autofill.credit_card_enabled", true);
  if (!download_dir.empty()) {
    ApplyStringPreference(manager, "download.default_directory", download_dir);
  }
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

void AppendTelemetry(const std::string& event,
                     const std::vector<std::pair<std::string, std::string>>& fields) {
  std::ostringstream payload;
  payload << "{\"source\":\"native_host\",\"event\":\"" << EscapeJsonString(event) << "\"";
  for (const auto& [key, value] : fields) {
    payload << ",\"" << EscapeJsonString(key) << "\":\"" << EscapeJsonString(value) << "\"";
  }
  payload << "}";
  AppendDebugLog(std::string("telemetry: ") + payload.str());
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

std::optional<std::string> StringKey(CefRefPtr<CefDictionaryValue> dict,
                                     const char* key) {
  if (!dict.get() || !dict->HasKey(key) || dict->GetType(key) != VTYPE_STRING) {
    return std::nullopt;
  }
  return dict->GetString(key).ToString();
}

std::optional<int> IntKey(CefRefPtr<CefDictionaryValue> dict, const char* key) {
  if (!dict.get() || !dict->HasKey(key)) {
    return std::nullopt;
  }
  const auto type = dict->GetType(key);
  if (type == VTYPE_INT) {
    return dict->GetInt(key);
  }
  if (type == VTYPE_DOUBLE) {
    return static_cast<int>(dict->GetDouble(key));
  }
  return std::nullopt;
}

std::optional<bool> BoolKey(CefRefPtr<CefDictionaryValue> dict, const char* key) {
  if (!dict.get() || !dict->HasKey(key) || dict->GetType(key) != VTYPE_BOOL) {
    return std::nullopt;
  }
  return dict->GetBool(key);
}

constexpr std::size_t kMaxWebSocketPayloadPreviewBytes = 4096;

struct PayloadPreview {
  std::string value;
  bool truncated = false;
};

PayloadPreview MakePayloadPreview(const std::string& payload) {
  if (payload.size() <= kMaxWebSocketPayloadPreviewBytes) {
    return {.value = payload, .truncated = false};
  }
  return {.value = payload.substr(0, kMaxWebSocketPayloadPreviewBytes), .truncated = true};
}

CefRefPtr<CefValue> WrapEvent(CefRefPtr<CefDictionaryValue> body) {
  auto event = CefDictionaryValue::Create();
  event->SetDictionary("event", body);

  auto wrapped = CefValue::Create();
  wrapped->SetDictionary(event);
  return wrapped;
}

CefRefPtr<CefValue> NavigationEventValue(const std::string& url) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "navigation");
  body->SetString("url", url);
  return WrapEvent(body);
}

CefRefPtr<CefValue> NetworkEventValue(const std::string& request_id,
                                      const std::string& url,
                                      const std::optional<std::string>& method,
                                      const std::optional<std::string>& resource_type,
                                      const std::optional<std::string>& phase,
                                      const std::optional<int>& status,
                                      const std::optional<std::string>& status_text,
                                      const std::optional<std::string>& mime_type,
                                      const std::optional<bool>& from_cache,
                                      const std::optional<std::string>& error_text) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "network");
  body->SetString("request_id", request_id);
  body->SetString("url", url);
  if (method.has_value()) {
    body->SetString("method", *method);
  }
  if (resource_type.has_value()) {
    body->SetString("resource_type", *resource_type);
  }
  if (phase.has_value()) {
    body->SetString("phase", *phase);
  }
  if (status.has_value() && *status >= 0) {
    body->SetInt("status", *status);
  }
  if (status_text.has_value()) {
    body->SetString("status_text", *status_text);
  }
  if (mime_type.has_value()) {
    body->SetString("mime_type", *mime_type);
  }
  if (from_cache.has_value()) {
    body->SetBool("from_cache", *from_cache);
  }
  if (error_text.has_value()) {
    body->SetString("error_text", *error_text);
  }
  return WrapEvent(body);
}

CefRefPtr<CefValue> DownloadEventValue(std::uint32_t id,
                                       const std::string& url,
                                       const std::string& suggested_name,
                                       const std::string& target_path,
                                       const std::string& mime_type,
                                       const std::string& state,
                                       std::uint64_t received_bytes,
                                       const std::optional<std::uint64_t>& total_bytes,
                                       const std::optional<int>& percent_complete,
                                       const std::optional<std::string>& interrupt_reason,
                                       bool complete,
                                       bool canceled) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "download");
  body->SetInt("id", static_cast<int>(id));
  if (!url.empty()) {
    body->SetString("url", url);
  }
  if (!suggested_name.empty()) {
    body->SetString("suggested_name", suggested_name);
  }
  if (!target_path.empty()) {
    body->SetString("target_path", target_path);
  }
  if (!mime_type.empty()) {
    body->SetString("mime_type", mime_type);
  }
  body->SetString("state", state);
  body->SetDouble("received_bytes", static_cast<double>(received_bytes));
  if (total_bytes.has_value()) {
    body->SetDouble("total_bytes", static_cast<double>(*total_bytes));
  }
  if (percent_complete.has_value()) {
    body->SetInt("percent_complete", *percent_complete);
  }
  if (interrupt_reason.has_value()) {
    body->SetString("interrupt_reason", *interrupt_reason);
  }
  body->SetBool("complete", complete);
  body->SetBool("canceled", canceled);
  return WrapEvent(body);
}

CefRefPtr<CefValue> WebSocketOpenEventValue(const std::string& request_id,
                                            const std::string& url) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "websocket_open");
  body->SetString("request_id", request_id);
  body->SetString("url", url);
  return WrapEvent(body);
}

CefRefPtr<CefValue> WebSocketHandshakeEventValue(
    const std::string& request_id,
    const std::string& url,
    const std::optional<int>& status,
    const std::optional<std::string>& status_text) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "websocket_handshake");
  body->SetString("request_id", request_id);
  body->SetString("url", url);
  if (status.has_value() && *status >= 0) {
    body->SetInt("status", *status);
  }
  if (status_text.has_value()) {
    body->SetString("status_text", *status_text);
  }
  return WrapEvent(body);
}

CefRefPtr<CefValue> WebSocketFrameEventValue(
    const std::string& request_id,
    const std::string& url,
    const char* direction,
    const std::optional<int>& opcode,
    const std::optional<bool>& mask,
    const std::string& payload) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "websocket_frame");
  body->SetString("request_id", request_id);
  body->SetString("url", url);
  body->SetString("direction", direction);
  if (opcode.has_value() && *opcode >= 0) {
    body->SetInt("opcode", *opcode);
  }
  if (mask.has_value()) {
    body->SetBool("mask", *mask);
  }
  const auto preview = MakePayloadPreview(payload);
  body->SetString("payload_preview", preview.value);
  body->SetInt("payload_length", static_cast<int>(payload.size()));
  body->SetBool("truncated", preview.truncated);
  return WrapEvent(body);
}

CefRefPtr<CefValue> WebSocketCloseEventValue(const std::string& request_id,
                                             const std::string& url) {
  auto body = CefDictionaryValue::Create();
  body->SetString("type", "websocket_close");
  body->SetString("request_id", request_id);
  body->SetString("url", url);
  return WrapEvent(body);
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
  std::optional<std::string> same_site;
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

std::string SanitizePathComponent(const std::string& value) {
  std::string output;
  output.reserve(value.size());
  for (char ch : value) {
    const auto byte = static_cast<unsigned char>(ch);
    if (std::isalnum(byte) != 0 || ch == '.' || ch == '-' || ch == '_') {
      output.push_back(ch);
    } else {
      output.push_back('_');
    }
  }
  output = TrimAscii(output);
  if (output.empty()) {
    return "download.bin";
  }
  return output;
}

std::filesystem::path UniqueDownloadTarget(const std::filesystem::path& directory,
                                           const std::string& suggested_name) {
  const auto safe_name = SanitizePathComponent(suggested_name.empty() ? "download.bin"
                                                                      : suggested_name);
  auto candidate = directory / safe_name;
  if (!std::filesystem::exists(candidate)) {
    return candidate;
  }

  const auto stem = candidate.stem().string();
  const auto extension = candidate.extension().string();
  for (std::size_t index = 1; index < 10'000; ++index) {
    candidate = directory / (stem + "-" + std::to_string(index) + extension);
    if (!std::filesystem::exists(candidate)) {
      return candidate;
    }
  }
  throw std::runtime_error("failed to allocate a unique download target path");
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
  std::string start_url = std::string(aegis::kBootstrapUrl);
  std::string download_dir;
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
      } else if (key == "download_dir") {
        options.download_dir = value;
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

std::optional<std::string> CookieSameSiteName(cef_cookie_same_site_t value) {
  switch (value) {
    case CEF_COOKIE_SAME_SITE_NO_RESTRICTION:
      return std::string("none");
    case CEF_COOKIE_SAME_SITE_LAX_MODE:
      return std::string("lax");
    case CEF_COOKIE_SAME_SITE_STRICT_MODE:
      return std::string("strict");
    case CEF_COOKIE_SAME_SITE_UNSPECIFIED:
    case CEF_COOKIE_SAME_SITE_NUM_VALUES:
      return std::nullopt;
  }
  return std::nullopt;
}

cef_cookie_same_site_t ParseCookieSameSite(const std::string& value) {
  if (CaseEqualAscii(value, "none")) {
    return CEF_COOKIE_SAME_SITE_NO_RESTRICTION;
  }
  if (CaseEqualAscii(value, "lax")) {
    return CEF_COOKIE_SAME_SITE_LAX_MODE;
  }
  if (CaseEqualAscii(value, "strict")) {
    return CEF_COOKIE_SAME_SITE_STRICT_MODE;
  }
  return CEF_COOKIE_SAME_SITE_UNSPECIFIED;
}

ManagedCookie ManagedCookieFromCef(const CefCookie& cookie) {
  const auto name = CefString(&cookie.name).ToString();
  const auto value = CefString(&cookie.value).ToString();
  const auto domain = CefString(&cookie.domain).ToString();
  const auto path = CefString(&cookie.path).ToString();
  ManagedCookie managed{
      .name = name,
      .value = value,
      .domain = domain,
      .path = path.empty() ? std::string("/") : path,
      .expires_unix = std::nullopt,
      .secure = cookie.secure != 0,
      .http_only = cookie.httponly != 0,
      .same_site = CookieSameSiteName(cookie.same_site),
  };
  if (cookie.has_expires != 0) {
    time_t expires = 0;
    cef_time_t expires_cef{};
    cef_time_from_basetime(cookie.expires, &expires_cef);
    if (cef_time_to_timet(&expires_cef, &expires) == 0) {
      managed.expires_unix = static_cast<std::uint64_t>(expires);
    }
  }
  return managed;
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

struct DownloadRecord {
  std::uint32_t id = 0;
  std::string url;
  std::string suggested_name;
  std::string target_path;
  std::string mime_type;
  std::string state = "pending";
  std::uint64_t received_bytes = 0;
  std::optional<std::uint64_t> total_bytes;
  std::optional<int> percent_complete;
  std::optional<std::string> interrupt_reason;
  bool complete = false;
  bool canceled = false;
};

struct BrowserContextState {
  std::string context_id = "primary";
  int browser_id = 0;
  bool page_ready = false;
  bool renderer_ready = false;
  bool runtime_ready = false;
  bool browser_closed = false;
  bool load_in_progress = false;
  std::string current_url = "about:blank";
  std::string last_committed_url = "about:blank";
  std::vector<std::pair<std::string, std::string>> network_overrides;
  std::vector<ManagedCookie> cookie_jar;
  std::vector<std::string> local_events;
  std::optional<std::string> pending_storage_injection_payload;
  std::map<std::string, std::string> request_urls;
  std::map<std::string, std::string> websocket_urls;
  std::map<int, RendererReply> renderer_replies;
  std::map<std::uint32_t, DownloadRecord> downloads;
  std::vector<std::uint32_t> download_order;
  CefRefPtr<CefBrowser> browser;
  CefRefPtr<CefRequestContext> request_context;

  void AttachBrowser(CefRefPtr<CefBrowser> browser) {
    this->browser = browser;
    browser_id = browser ? browser->GetIdentifier() : 0;
    browser_closed = false;
    page_ready = false;
    renderer_ready = false;
    runtime_ready = false;
    load_in_progress = browser ? browser->IsLoading() : false;
    if (browser && browser->GetMainFrame()) {
      current_url = browser->GetMainFrame()->GetURL().ToString();
      if (!current_url.empty()) {
        last_committed_url = current_url;
      }
    }
    request_context =
        browser && browser->GetHost() ? browser->GetHost()->GetRequestContext() : nullptr;
  }

  void BeginNavigation(const std::string& url) {
    current_url = url;
    page_ready = false;
    renderer_ready = false;
    runtime_ready = false;
    load_in_progress = true;
  }

  void MarkPageLoad(bool is_loading, const std::string& url) {
    page_ready = !is_loading;
    load_in_progress = is_loading;
    if (!url.empty()) {
      current_url = url;
      if (!is_loading) {
        last_committed_url = url;
      }
    }
  }

  void MarkLifecycleReady(const std::string& url) {
    renderer_ready = true;
    runtime_ready = true;
    if (!url.empty()) {
      current_url = url;
    }
  }

  void RestoreAfterDownload() {
    page_ready = true;
    renderer_ready = true;
    runtime_ready = true;
    load_in_progress = false;
    if (!last_committed_url.empty()) {
      current_url = last_committed_url;
    }
  }

  void InvalidateRuntime(bool invalidate_renderer) {
    runtime_ready = false;
    if (invalidate_renderer) {
      renderer_ready = false;
    }
  }

  void DetachBrowser() {
    browser = nullptr;
    request_context = nullptr;
    browser_id = 0;
    page_ready = false;
    renderer_ready = false;
    runtime_ready = false;
    browser_closed = true;
    load_in_progress = false;
    request_urls.clear();
    websocket_urls.clear();
    renderer_replies.clear();
  }

  void UpsertDownload(const DownloadRecord& record) {
    downloads[record.id] = record;
    if (std::find(download_order.begin(), download_order.end(), record.id) ==
        download_order.end()) {
      download_order.push_back(record.id);
    }
    while (download_order.size() > 64) {
      const auto removed = download_order.front();
      download_order.erase(download_order.begin());
      downloads.erase(removed);
    }
  }
};

class AegisCefHost;

class AegisDevToolsObserver final : public CefDevToolsMessageObserver {
 public:
  explicit AegisDevToolsObserver(AegisCefHost* host) : host_(host) {}

  void OnDevToolsEvent(CefRefPtr<CefBrowser> browser,
                       const CefString& method,
                       const void* params,
                       size_t params_size) override;

  void OnDevToolsAgentAttached(CefRefPtr<CefBrowser> browser) override;

  void OnDevToolsAgentDetached(CefRefPtr<CefBrowser> browser) override;

 private:
  AegisCefHost* host_;

  IMPLEMENT_REFCOUNTING(AegisDevToolsObserver);
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
  friend class AegisDevToolsObserver;

  explicit AegisCefHost(BrowserOptions options,
                        bool manage_cef_lifecycle = true,
                        bool counted_shared_lifecycle = false)
      : options_(std::move(options)),
        download_dir_(
            options_.download_dir.empty()
                ? AegisDownloadsRoot()
                : std::filesystem::path(options_.download_dir)),
        paths_(ResolveHostPaths()),
        runtime_session_paths_(AegisCreateRuntimeSessionPaths(
            options_.headless ? "serve-headless" : "serve-headful")),
        owner_thread_id_(std::this_thread::get_id()),
        manage_cef_lifecycle_(manage_cef_lifecycle),
        counted_shared_lifecycle_(counted_shared_lifecycle) {
    if (pthread_main_np() == 0) {
      throw std::runtime_error("aegis CEF host must be created on the process main thread");
    }
    std::error_code error;
    std::filesystem::create_directories(download_dir_, error);
    if (error) {
      throw std::runtime_error("failed to create download directory: " +
                               download_dir_.string());
    }
    AppendDebugLog("host: constructed");
    if (manage_cef_lifecycle_) {
      Start();
    } else {
      AttachToInitializedCef();
    }
  }

  ~AegisCefHost() override {
    bool shutdown_cef = manage_cef_lifecycle_;
    if (counted_shared_lifecycle_) {
      std::lock_guard lock(g_shared_host_lifecycle_mutex);
      if (g_shared_host_count > 0) {
        --g_shared_host_count;
      }
      shutdown_cef = g_shared_host_count == 0;
    }
    Shutdown(shutdown_cef);
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

  std::vector<std::uint8_t> EnsureRuntime(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "ensure_runtime");
    try {
      SetOperationStage("ensuring runtime is ready");
      EnsureRuntimeReady();
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
      SetOperationStage("ensuring runtime is ready");
      EnsureRuntimeReady();
      SetOperationStage("decoding batch request");
      auto payload =
          RequireDictionary(DecodeEnvelope(MessageKind::SendBatch, request),
                            "batch request must be a dictionary");
      if (BoolKey(payload, "capture_network_events").value_or(false)) {
        SetOperationStage("enabling network event capture");
        EnsureNetworkEventCapture();
      }
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
      SetOperationStage("ensuring runtime is ready");
      EnsureRuntimeReady();
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
      const auto payload_json = WriteJson(payload);
      {
        std::lock_guard lock(mutex_);
        primary_context_.pending_storage_injection_payload = payload_json;
      }
      if (!TryApplyPendingStorageInjection()) {
        AppendDebugLog("host: deferred_storage_injection awaiting renderer readiness");
      }
      return {};
    } catch (const std::exception& error) {
      throw std::runtime_error(WrapOperationError(error.what()));
    }
  }

  std::vector<std::uint8_t> SnapshotSession(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "snapshot_session");
    try {
      SetOperationStage("ensuring runtime is ready");
      EnsureRuntimeReady();
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
    OperationScope scope(this, "drain_events");
    try {
      auto payload = RequireDictionary(
          DecodeEnvelope(MessageKind::DrainEvents, request),
          "drain events request must be a dictionary");
      if (BoolKey(payload, "enable_network_capture").value_or(false)) {
        SetOperationStage("enabling network event capture");
        EnsureNetworkEventCapture();
      }
      bool renderer_ready = false;
      bool runtime_ready = false;
      {
        std::lock_guard lock(mutex_);
        renderer_ready = primary_context_.renderer_ready;
        runtime_ready = primary_context_.runtime_ready;
      }
      if (!renderer_ready || !runtime_ready) {
        auto response = CefDictionaryValue::Create();
        response->SetList("events", CefListValue::Create());
        MergeLocalEventsIntoResponse(response);
        return EncodeEnvelope(MessageKind::DrainEvents, response);
      }

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
      if (BoolKey(payload, "capture_network_events").value_or(false)) {
        SetOperationStage("enabling network event capture");
        EnsureNetworkEventCapture();
      }
      const auto target_url = payload->GetString("url").ToString();
      SetOperationStage("starting browser navigation");
      NavigateTo(target_url);
      SetOperationStage("capturing navigation state");

      auto response = CefDictionaryValue::Create();
      response->SetString("url", CurrentUrl());

      auto events = CefListValue::Create();
      events->SetValue(0, NavigationEventValue(CurrentUrl()));
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

  std::vector<std::uint8_t> SnapshotHostState(const std::vector<std::uint8_t>& request) override {
    static_cast<void>(request);
    OperationScope scope(this, "snapshot_host_state");
    auto state = CefDictionaryValue::Create();
    {
      std::lock_guard lock(mutex_);
      const auto* active_context =
          ActiveContextStateLocked() != nullptr ? ActiveContextStateLocked() : &primary_context_;
      state->SetBool("startup_complete", startup_complete_);
      state->SetBool("browser_available", browser_.get() != nullptr);
      state->SetString("active_context_id", active_context->context_id);
      state->SetInt("active_browser_id", active_browser_id_);
      state->SetString("context_id", active_context->context_id);
      state->SetInt("browser_id", active_context->browser_id);
      state->SetBool("request_context_available", active_context->request_context.get() != nullptr);
      state->SetBool("page_ready", active_context->page_ready);
      state->SetBool("renderer_ready", active_context->renderer_ready);
      state->SetBool("runtime_installed", active_context->renderer_ready);
      state->SetBool("runtime_ready", active_context->runtime_ready);
      state->SetBool("load_in_progress", active_context->load_in_progress);
      state->SetBool("browser_closed", active_context->browser_closed);
      state->SetBool("cancel_requested", cancel_requested_.load());
      state->SetString("download_dir", download_dir_.string());
      if (!active_context->current_url.empty()) {
        state->SetString("current_url", active_context->current_url);
      }
      if (!current_operation_name_.empty()) {
        state->SetString("active_operation", current_operation_name_);
      }
      if (!current_operation_stage_.empty()) {
        state->SetString("active_stage", current_operation_stage_);
      }
      auto attached_browser_ids = CefListValue::Create();
      auto known_context_ids = CefListValue::Create();
      int index = 0;
      for (const auto& [browser_id, context] : browser_contexts_) {
        attached_browser_ids->SetInt(index, browser_id);
        known_context_ids->SetString(index, context.context_id);
        index += 1;
      }
      state->SetList("attached_browser_ids", attached_browser_ids);
      state->SetList("known_context_ids", known_context_ids);
      auto downloads = CefListValue::Create();
      int download_index = 0;
      for (const auto download_id : active_context->download_order) {
        const auto it = active_context->downloads.find(download_id);
        if (it == active_context->downloads.end()) {
          continue;
        }
        const auto& record = it->second;
        auto entry = CefDictionaryValue::Create();
        entry->SetInt("id", static_cast<int>(record.id));
        if (!record.url.empty()) {
          entry->SetString("url", record.url);
        }
        if (!record.suggested_name.empty()) {
          entry->SetString("suggested_name", record.suggested_name);
        }
        if (!record.target_path.empty()) {
          entry->SetString("target_path", record.target_path);
        }
        if (!record.mime_type.empty()) {
          entry->SetString("mime_type", record.mime_type);
        }
        entry->SetString("state", record.state);
        entry->SetDouble("received_bytes", static_cast<double>(record.received_bytes));
        if (record.total_bytes.has_value()) {
          entry->SetDouble("total_bytes", static_cast<double>(*record.total_bytes));
        }
        if (record.percent_complete.has_value()) {
          entry->SetInt("percent_complete", *record.percent_complete);
        }
        if (record.interrupt_reason.has_value()) {
          entry->SetString("interrupt_reason", *record.interrupt_reason);
        }
        entry->SetBool("complete", record.complete);
        entry->SetBool("canceled", record.canceled);
        downloads->SetDictionary(download_index++, entry);
      }
      state->SetList("downloads", downloads);
    }
    return EncodeEnvelope(MessageKind::SnapshotHostState, state);
  }

  std::vector<std::uint8_t> ActivateBrowser(const std::vector<std::uint8_t>& request) override {
    OperationScope scope(this, "activate_browser");
    try {
      SetOperationStage("decoding browser activation request");
      auto payload = RequireDictionary(
          DecodeEnvelope(MessageKind::ActivateBrowser, request),
          "activate browser request must be a dictionary");
      if (!payload->HasKey("browser_id")) {
        throw std::runtime_error("activate browser request must include browser_id");
      }
      const auto browser_id = payload->GetInt("browser_id");
      SetOperationStage("switching active browser");
      ActivateAttachedBrowser(browser_id);
      SetOperationStage("capturing activated browser state");
      return SnapshotHostState({});
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

  void RequestCancel() override {
    cancel_requested_.store(true);
    std::lock_guard lock(mutex_);
    cv_.notify_all();
  }

  void OnPrimaryBrowserCreated(CefRefPtr<CefBrowser> browser) override {
    AppendDebugLog("host: on_browser_created");
    {
      std::lock_guard lock(mutex_);
      auto& context = EnsureContextStateLocked(browser);
      context.AttachBrowser(browser);
      if (!browser_.get()) {
        browser_ = browser;
        request_context_ = context.request_context;
        primary_context_ = context;
        SyncPrimaryContextToRegistryLocked();
      }
      cv_.notify_all();
    }
    AppendTelemetry("primary_browser_created",
                    {{"browser_id",
                      std::to_string(browser ? browser->GetIdentifier() : 0)},
                     {"url",
                      browser && browser->GetMainFrame() ? browser->GetMainFrame()->GetURL().ToString()
                                                         : std::string()},
                     {"thread", ThreadLabel()}});
    if (!options_.headless && browser && AegisUseExternalBrowserHostWindow()) {
      AegisSetBrowserHostAddress(browser->GetMainFrame()->GetURL().ToString());
      AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                         browser->IsLoading());
      AegisAttachBrowserToHostWindow(browser);
      AegisShowBrowserHostWindow();
    }
  }

  void OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                      CefRefPtr<CefFrame> frame,
                      CefRefPtr<CefRequest> request) override {
    AppendDebugLog("host: on_before_browse");
    std::lock_guard lock(mutex_);
    if (!frame.get() || !frame->IsMain()) {
      return;
    }
    auto& context = EnsureContextStateLocked(browser);
    context.page_ready = false;
    context.renderer_ready = false;
    context.runtime_ready = false;
    context.load_in_progress = true;
    if (request.get()) {
      context.current_url = request->GetURL().ToString();
    } else if (frame.get()) {
      context.current_url = frame->GetURL().ToString();
    }
    if (browser_.get() && browser.get() && browser->IsSame(browser_)) {
      primary_context_ = context;
      SyncPrimaryContextToRegistryLocked();
    }
  }

  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser, bool is_loading) override {
    AppendDebugLog(std::string("host: on_loading_state_change loading=") +
                   (is_loading ? "true" : "false"));
    {
      std::lock_guard lock(mutex_);
      auto* context = ContextStateForBrowserLocked(browser);
      if (context == nullptr) {
        return;
      }
      context->page_ready = !is_loading;
      context->load_in_progress = is_loading;
      if (!is_loading) {
        context->current_url = browser->GetMainFrame()->GetURL().ToString();
      }
      if (browser_.get() && browser->IsSame(browser_)) {
        primary_context_ = *context;
        SyncPrimaryContextToRegistryLocked();
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
    auto* context = ContextStateForBrowserLocked(browser);
    if (context == nullptr) {
      return;
    }
    context->current_url = frame->GetURL().ToString();
    if (!context->current_url.empty()) {
      context->last_committed_url = context->current_url;
    }
    if (browser_.get() && browser->IsSame(browser_)) {
      primary_context_ = *context;
      SyncPrimaryContextToRegistryLocked();
    }
  }

  void OnAddressChange(CefRefPtr<CefBrowser>,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override {
    if (options_.headless || !frame.get() || !frame->IsMain()) {
      return;
    }
    AegisSetBrowserHostAddress(url.ToString());
  }

  void OnTitleChange(CefRefPtr<CefBrowser>,
                     const CefString& title) override {
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
        const auto browser_id = browser->GetIdentifier();
        browser_ = nullptr;
        request_context_ = nullptr;
        client_ = nullptr;
        primary_context_.DetachBrowser();
        browser_contexts_.erase(browser_id);
        active_browser_id_ = 0;
        devtools_registration_ = nullptr;
        devtools_network_enabled_ = false;
        cv_.notify_all();
      }
    }
    if (!options_.headless) {
      AegisCloseBrowserHostWindow();
    }
  }

  cef_return_value_t OnBeforeResourceLoad(CefRefPtr<CefBrowser>,
                                          CefRefPtr<CefFrame>,
                                          CefRefPtr<CefRequest> request) override {
    std::lock_guard lock(mutex_);
    if (primary_context_.network_overrides.empty()) {
      return RV_CONTINUE;
    }

    CefRequest::HeaderMap headers;
    request->GetHeaderMap(headers);
    for (const auto& [header, value] : primary_context_.network_overrides) {
      auto range = headers.equal_range(header);
      headers.erase(range.first, range.second);
      headers.emplace(header, value);
    }
    request->SetHeaderMap(headers);
    return RV_CONTINUE;
  }

  void OnResourceLoadComplete(CefRefPtr<CefBrowser>,
                              CefRefPtr<CefFrame>,
                              CefRefPtr<CefRequest> request,
                              CefRefPtr<CefResponse> response,
                              cef_urlrequest_status_t) override {
    CaptureResponseCookies(request, response);
  }

  bool OnBeforeDownload(CefRefPtr<CefBrowser> browser,
                        CefRefPtr<CefDownloadItem> download_item,
                        const CefString& suggested_name,
                        CefRefPtr<CefBeforeDownloadCallback> callback) override {
    if (!download_item.get() || !callback.get()) {
      return false;
    }

    std::filesystem::path target_path;
    DownloadRecord record;
    {
      std::lock_guard lock(mutex_);
      auto& context = EnsureContextStateLocked(browser);
      const auto directory = download_dir_;
      std::error_code error;
      std::filesystem::create_directories(directory, error);
      if (error) {
        throw std::runtime_error("failed to create download directory: " +
                                 directory.string());
      }
      target_path = UniqueDownloadTarget(directory, suggested_name.ToString());
      record.id = download_item->GetId();
      record.url = download_item->GetURL().ToString();
      record.suggested_name = suggested_name.ToString();
      record.target_path = target_path.string();
      record.mime_type = download_item->GetMimeType().ToString();
      record.state = "starting";
      record.received_bytes = static_cast<std::uint64_t>(download_item->GetReceivedBytes());
      if (download_item->GetTotalBytes() >= 0) {
        record.total_bytes =
            static_cast<std::uint64_t>(download_item->GetTotalBytes());
      }
      if (download_item->GetPercentComplete() >= 0) {
        record.percent_complete = download_item->GetPercentComplete();
      }
      context.RestoreAfterDownload();
      context.UpsertDownload(record);
      if (browser_.get() && browser.get() && browser->IsSame(browser_)) {
        primary_context_ = context;
        SyncPrimaryContextToRegistryLocked();
      }
      cv_.notify_all();
    }

    PushLocalEvent(DownloadEventValue(record.id, record.url, record.suggested_name,
                                      record.target_path, record.mime_type, record.state,
                                      record.received_bytes, record.total_bytes,
                                      record.percent_complete, record.interrupt_reason,
                                      record.complete, record.canceled));
    callback->Continue(target_path.string(), false);
    if (!options_.headless && browser.get()) {
      AegisSetBrowserHostAddress(browser->GetMainFrame()->GetURL().ToString());
    }
    return true;
  }

  void OnDownloadUpdated(CefRefPtr<CefBrowser> browser,
                         CefRefPtr<CefDownloadItem> download_item,
                         CefRefPtr<CefDownloadItemCallback>) override {
    if (!download_item.get()) {
      return;
    }

    DownloadRecord record;
    {
      std::lock_guard lock(mutex_);
      auto* context = ContextStateForBrowserLocked(browser);
      if (context == nullptr) {
        return;
      }
      const auto id = download_item->GetId();
      auto existing = context->downloads.find(id);
      if (existing != context->downloads.end()) {
        record = existing->second;
      }
      record.id = id;
      record.url = download_item->GetURL().ToString();
      if (record.suggested_name.empty()) {
        record.suggested_name = download_item->GetSuggestedFileName().ToString();
      }
      const auto full_path = download_item->GetFullPath().ToString();
      if (!full_path.empty()) {
        record.target_path = full_path;
      }
      const auto mime_type = download_item->GetMimeType().ToString();
      if (!mime_type.empty()) {
        record.mime_type = mime_type;
      }
      record.received_bytes =
          static_cast<std::uint64_t>(download_item->GetReceivedBytes());
      if (download_item->GetTotalBytes() >= 0) {
        record.total_bytes =
            static_cast<std::uint64_t>(download_item->GetTotalBytes());
      }
      if (download_item->GetPercentComplete() >= 0) {
        record.percent_complete = download_item->GetPercentComplete();
      } else {
        record.percent_complete.reset();
      }
      const auto interrupt_reason =
          static_cast<int>(download_item->GetInterruptReason());
      if (interrupt_reason != 0) {
        record.interrupt_reason = std::to_string(interrupt_reason);
      }
      record.complete = download_item->IsComplete();
      record.canceled = download_item->IsCanceled();
      if (record.complete) {
        record.state = "completed";
      } else if (record.canceled) {
        record.state = "canceled";
      } else if (record.interrupt_reason.has_value()) {
        record.state = "interrupted";
      } else if (download_item->IsInProgress()) {
        record.state = "in_progress";
      } else {
        record.state = "pending";
      }
      context->UpsertDownload(record);
      if (browser_.get() && browser.get() && browser->IsSame(browser_)) {
        primary_context_ = *context;
        SyncPrimaryContextToRegistryLocked();
      }
    }

    PushLocalEvent(DownloadEventValue(record.id, record.url, record.suggested_name,
                                      record.target_path, record.mime_type, record.state,
                                      record.received_bytes, record.total_bytes,
                                      record.percent_complete, record.interrupt_reason,
                                      record.complete, record.canceled));
  }

  bool HandleBrowserProcessMessage(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefProcessId source_process,
                                   CefRefPtr<CefProcessMessage> message) {
    if (source_process != PID_RENDERER || !message.get() ||
        (message->GetName() != aegis::kAegisResponseMessage &&
         message->GetName() != aegis::kAegisLifecycleMessage)) {
      return false;
    }

    if (message->GetName() == aegis::kAegisLifecycleMessage) {
      if (!frame.get() || !frame->IsMain()) {
        return false;
      }
      auto args = message->GetArgumentList();
      if (args->GetString(0).ToString() == aegis::kLifecycleContextReady) {
        AppendDebugLog("host: lifecycle context_ready");
        {
          std::lock_guard lock(mutex_);
          auto& context = EnsureContextStateLocked(browser);
          context.MarkLifecycleReady(args->GetString(1).ToString());
          if (browser_.get() && browser.get() && browser->IsSame(browser_)) {
            primary_context_ = context;
            SyncPrimaryContextToRegistryLocked();
          }
          cv_.notify_all();
        }
        return true;
      }
      return false;
    }

    auto args = message->GetArgumentList();
    if (!frame.get() || !frame->IsMain()) {
      return false;
    }
    AppendDebugLog("host: renderer response received");
    CompleteRendererRequest(args->GetInt(0), args->GetBool(1),
                            args->GetString(2).ToString());
    return true;
  }

 private:
  void BeginOperation(const std::string& name) {
    cancel_requested_.store(false);
    current_operation_name_ = name;
    current_operation_stage_ = "starting";
    current_operation_started_at_ = std::chrono::steady_clock::now();
  }

  void EndOperation() {
    cancel_requested_.store(false);
    current_operation_name_.clear();
    current_operation_stage_.clear();
  }

  void SetOperationStage(const std::string& stage) { current_operation_stage_ = stage; }

  BrowserContextState* ContextStateForBrowserLocked(CefRefPtr<CefBrowser> browser) {
    if (!browser.get()) {
      return nullptr;
    }
    auto found = browser_contexts_.find(browser->GetIdentifier());
    if (found == browser_contexts_.end()) {
      return nullptr;
    }
    return &found->second;
  }

  const BrowserContextState* ContextStateForBrowserLocked(CefRefPtr<CefBrowser> browser) const {
    if (!browser.get()) {
      return nullptr;
    }
    auto found = browser_contexts_.find(browser->GetIdentifier());
    if (found == browser_contexts_.end()) {
      return nullptr;
    }
    return &found->second;
  }

  BrowserContextState* ActiveContextStateLocked() {
    auto found = browser_contexts_.find(active_browser_id_);
    if (found == browser_contexts_.end()) {
      return nullptr;
    }
    return &found->second;
  }

  const BrowserContextState* ActiveContextStateLocked() const {
    auto found = browser_contexts_.find(active_browser_id_);
    if (found == browser_contexts_.end()) {
      return nullptr;
    }
    return &found->second;
  }

  BrowserContextState& EnsureContextStateLocked(CefRefPtr<CefBrowser> browser,
                                                const std::string& context_id = "primary") {
    const auto browser_id = browser ? browser->GetIdentifier() : 0;
    auto [it, inserted] = browser_contexts_.try_emplace(browser_id);
    if (inserted) {
      it->second.context_id = context_id;
    }
    if (browser.get()) {
      it->second.browser = browser;
      it->second.request_context =
          browser->GetHost() ? browser->GetHost()->GetRequestContext() : nullptr;
      it->second.browser_id = browser_id;
    }
    return it->second;
  }

  void SyncPrimaryContextToRegistryLocked() {
    if (primary_context_.browser_id <= 0) {
      return;
    }
    browser_contexts_[primary_context_.browser_id] = primary_context_;
    active_browser_id_ = primary_context_.browser_id;
  }

  void SyncRegistryToPrimaryContextLocked(int browser_id) {
    auto found = browser_contexts_.find(browser_id);
    if (found == browser_contexts_.end()) {
      return;
    }
    primary_context_ = found->second;
    active_browser_id_ = browser_id;
  }

  std::vector<int> AttachedBrowserIdsLocked() const {
    std::vector<int> browser_ids;
    browser_ids.reserve(browser_contexts_.size());
    for (const auto& [browser_id, _state] : browser_contexts_) {
      browser_ids.push_back(browser_id);
    }
    return browser_ids;
  }

  bool IsManagedBrowser(CefRefPtr<CefBrowser> browser) const {
    std::lock_guard lock(mutex_);
    return ContextStateForBrowserLocked(browser) != nullptr;
  }

  void ActivateAttachedBrowser(int browser_id) {
    RequireOwnerThread();
    std::lock_guard lock(mutex_);
    auto found = browser_contexts_.find(browser_id);
    if (found == browser_contexts_.end()) {
      throw std::runtime_error("attached browser was not found");
    }
    if (!found->second.browser.get()) {
      throw std::runtime_error("attached browser is no longer available");
    }
    primary_context_ = found->second;
    browser_ = found->second.browser;
    request_context_ = found->second.request_context;
    active_browser_id_ = browser_id;
    devtools_registration_ = nullptr;
    devtools_network_enabled_ = false;
    cv_.notify_all();
  }

  void EnsureDevToolsObserver(CefRefPtr<CefBrowser> browser) {
    if (!browser.get()) {
      return;
    }
    if (devtools_registration_.get()) {
      EnableDevToolsNetworkTracking(browser);
      return;
    }
    auto observer = new AegisDevToolsObserver(this);
    devtools_registration_ = browser->GetHost()->AddDevToolsMessageObserver(observer);
    if (!devtools_registration_.get()) {
      throw std::runtime_error("failed to register DevTools observer");
    }
    EnableDevToolsNetworkTracking(browser);
  }

  void EnsureNetworkEventCapture() {
    RequireOwnerThread();
    EnsureBrowserAvailable();
    RunOnUiThreadSync([this]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      EnsureDevToolsObserver(browser_);
    });
  }

  void EnableDevToolsNetworkTracking(CefRefPtr<CefBrowser> browser) {
    if (!browser.get()) {
      return;
    }
    auto params = CefDictionaryValue::Create();
    params->SetInt("maxTotalBufferSize", 16 * 1024 * 1024);
    params->SetInt("maxResourceBufferSize", 4 * 1024 * 1024);
    if (browser->GetHost()->ExecuteDevToolsMethod(0, "Network.enable", params) == 0) {
      throw std::runtime_error("failed to enable DevTools Network domain");
    }
    devtools_network_enabled_ = true;
  }

  void HandleDevToolsAgentAttached(CefRefPtr<CefBrowser> browser) {
    if (!IsManagedBrowser(browser)) {
      return;
    }
    AppendDebugLog("host: devtools agent attached");
    try {
      EnableDevToolsNetworkTracking(browser);
    } catch (const std::exception& error) {
      AppendDebugLog(std::string("host: failed_to_enable_devtools_network ") + error.what());
    }
  }

  void HandleDevToolsAgentDetached(CefRefPtr<CefBrowser> browser) {
    if (!IsManagedBrowser(browser)) {
      return;
    }
    AppendDebugLog("host: devtools agent detached");
    devtools_network_enabled_ = false;
  }

  std::optional<std::string> UrlForRequestId(const std::string& request_id) const {
    std::lock_guard lock(mutex_);
    auto found = primary_context_.request_urls.find(request_id);
    if (found == primary_context_.request_urls.end()) {
      return std::nullopt;
    }
    return found->second;
  }

  void RememberRequestUrl(const std::string& request_id, const std::string& url) {
    std::lock_guard lock(mutex_);
    primary_context_.request_urls[request_id] = url;
    SyncPrimaryContextToRegistryLocked();
  }

  void ForgetRequestUrl(const std::string& request_id) {
    std::lock_guard lock(mutex_);
    primary_context_.request_urls.erase(request_id);
    SyncPrimaryContextToRegistryLocked();
  }

  void RememberWebSocketUrl(const std::string& request_id, const std::string& url) {
    std::lock_guard lock(mutex_);
    primary_context_.websocket_urls[request_id] = url;
    SyncPrimaryContextToRegistryLocked();
  }

  std::optional<std::string> WebSocketUrl(const std::string& request_id) const {
    std::lock_guard lock(mutex_);
    auto found = primary_context_.websocket_urls.find(request_id);
    if (found == primary_context_.websocket_urls.end()) {
      return std::nullopt;
    }
    return found->second;
  }

  void ForgetWebSocketUrl(const std::string& request_id) {
    std::lock_guard lock(mutex_);
    primary_context_.websocket_urls.erase(request_id);
    SyncPrimaryContextToRegistryLocked();
  }

  void HandleDevToolsEvent(CefRefPtr<CefBrowser> browser,
                           const CefString& method,
                           const void* params,
                           size_t params_size) {
    if (!IsManagedBrowser(browser)) {
      return;
    }

    try {
      const auto method_name = method.ToString();
      const auto params_json =
          params != nullptr && params_size > 0
              ? std::string(static_cast<const char*>(params), params_size)
              : std::string("{}");
      auto params_dict = RequireDictionary(
          ParseJsonValue(params_json,
                         "devtools event params are not valid json"),
          "devtools event params must be a dictionary");

      if (method_name == "Network.requestWillBeSent") {
        const auto request_id = StringKey(params_dict, "requestId");
        const auto resource_type = StringKey(params_dict, "type");
        auto request = params_dict->HasKey("request") ? params_dict->GetDictionary("request")
                                                       : CefDictionaryValue::Create();
        const auto url = StringKey(request, "url");
        const auto method = StringKey(request, "method");
        if (request_id.has_value() && url.has_value()) {
          RememberRequestUrl(*request_id, *url);
          if (resource_type != std::optional<std::string>("WebSocket")) {
            PushLocalEvent(NetworkEventValue(*request_id, *url, method, resource_type,
                                             std::string("request"), std::nullopt,
                                             std::nullopt, std::nullopt, std::nullopt,
                                             std::nullopt));
          }
        }
        return;
      }

      if (method_name == "Network.responseReceived") {
        const auto request_id = StringKey(params_dict, "requestId");
        const auto resource_type = StringKey(params_dict, "type");
        auto response = params_dict->HasKey("response") ? params_dict->GetDictionary("response")
                                                         : CefDictionaryValue::Create();
        const auto url = StringKey(response, "url");
        const auto status = IntKey(response, "status");
        const auto status_text = StringKey(response, "statusText");
        const auto mime_type = StringKey(response, "mimeType");
        const auto from_disk_cache = BoolKey(response, "fromDiskCache");
        const auto from_prefetch_cache = BoolKey(response, "fromPrefetchCache");
        const auto from_service_worker = BoolKey(response, "fromServiceWorker");
        std::optional<bool> from_cache;
        if (from_disk_cache.value_or(false) || from_prefetch_cache.value_or(false) ||
            from_service_worker.value_or(false)) {
          from_cache = true;
        }
        if (request_id.has_value() && url.has_value() &&
            resource_type != std::optional<std::string>("WebSocket")) {
          PushLocalEvent(NetworkEventValue(*request_id, *url, std::nullopt, resource_type,
                                           std::string("response"), status, status_text,
                                           mime_type, from_cache, std::nullopt));
        }
        return;
      }

      if (method_name == "Network.loadingFinished") {
        const auto request_id = StringKey(params_dict, "requestId");
        if (request_id.has_value()) {
          if (const auto url = UrlForRequestId(*request_id); url.has_value()) {
            PushLocalEvent(NetworkEventValue(*request_id, *url, std::nullopt, std::nullopt,
                                             std::string("finished"), std::nullopt,
                                             std::nullopt, std::nullopt, std::nullopt,
                                             std::nullopt));
          }
          ForgetRequestUrl(*request_id);
        }
        return;
      }

      if (method_name == "Network.loadingFailed") {
        const auto request_id = StringKey(params_dict, "requestId");
        const auto error_text = StringKey(params_dict, "errorText");
        if (request_id.has_value()) {
          if (const auto url = UrlForRequestId(*request_id); url.has_value()) {
            PushLocalEvent(NetworkEventValue(*request_id, *url, std::nullopt, std::nullopt,
                                             std::string("failed"), std::nullopt,
                                             std::nullopt, std::nullopt, std::nullopt,
                                             error_text));
          }
          ForgetRequestUrl(*request_id);
        }
        return;
      }

      if (method_name == "Network.webSocketCreated") {
        const auto request_id = StringKey(params_dict, "requestId");
        const auto url = StringKey(params_dict, "url");
        if (request_id.has_value() && url.has_value()) {
          RememberWebSocketUrl(*request_id, *url);
          PushLocalEvent(WebSocketOpenEventValue(*request_id, *url));
        }
        return;
      }

      if (method_name == "Network.webSocketHandshakeResponseReceived") {
        const auto request_id = StringKey(params_dict, "requestId");
        auto response = params_dict->HasKey("response") ? params_dict->GetDictionary("response")
                                                         : CefDictionaryValue::Create();
        if (request_id.has_value()) {
          const auto url = WebSocketUrl(*request_id).value_or(std::string());
          PushLocalEvent(WebSocketHandshakeEventValue(*request_id, url,
                                                      IntKey(response, "status"),
                                                      StringKey(response, "statusText")));
        }
        return;
      }

      if (method_name == "Network.webSocketFrameSent" ||
          method_name == "Network.webSocketFrameReceived") {
        const auto request_id = StringKey(params_dict, "requestId");
        auto response = params_dict->HasKey("response") ? params_dict->GetDictionary("response")
                                                         : CefDictionaryValue::Create();
        if (request_id.has_value()) {
          const auto url = WebSocketUrl(*request_id).value_or(std::string());
          PushLocalEvent(WebSocketFrameEventValue(
              *request_id, url,
              method_name == "Network.webSocketFrameSent" ? "sent" : "received",
              IntKey(response, "opcode"), BoolKey(response, "mask"),
              StringKey(response, "payloadData").value_or(std::string())));
        }
        return;
      }

      if (method_name == "Network.webSocketClosed") {
        const auto request_id = StringKey(params_dict, "requestId");
        if (request_id.has_value()) {
          const auto url = WebSocketUrl(*request_id).value_or(std::string());
          PushLocalEvent(WebSocketCloseEventValue(*request_id, url));
          ForgetWebSocketUrl(*request_id);
        }
      }
    } catch (const std::exception& error) {
      AppendDebugLog(std::string("host: devtools event parse error ") + error.what());
    }
  }

  std::string WrapOperationError(const std::string& message) const {
    if (current_operation_name_.empty() || IsStructuredOperationError(message)) {
      return message;
    }
    const auto elapsed_ms = static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now() - current_operation_started_at_)
            .count());
    const bool timed_out = message.find("timed out") != std::string::npos;
    const bool cancelled = message.find("operation cancelled") != std::string::npos;
    const bool restart_recommended =
        !cancelled && (timed_out || message.find("browser window closed by user") != std::string::npos ||
        message.find("browser is not available") != std::string::npos);
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
    const auto started_at = std::chrono::steady_clock::now();
    auto last_wait_log_at = started_at;
    for (;;) {
      RequireOwnerThread();

      {
        std::lock_guard lock(mutex_);
        if (!startup_error_.empty()) {
          throw std::runtime_error(startup_error_);
        }
        if (cancel_requested_.load()) {
          throw std::runtime_error("operation cancelled");
        }
        if (predicate()) {
          return;
        }
      }

      if (std::chrono::steady_clock::now() >= deadline) {
        throw std::runtime_error(timeout_message);
      }

      const auto now = std::chrono::steady_clock::now();
      if (now - last_wait_log_at >= std::chrono::milliseconds(500)) {
        std::lock_guard lock(mutex_);
        AppendTelemetry(
            "pump_until_waiting",
            {{"message", timeout_message},
             {"elapsed_ms",
              std::to_string(
                  std::chrono::duration_cast<std::chrono::milliseconds>(now - started_at).count())},
             {"startup_complete", startup_complete_ ? "true" : "false"},
             {"browser_available", browser_.get() != nullptr ? "true" : "false"},
             {"context_id", primary_context_.context_id},
             {"browser_id", std::to_string(primary_context_.browser_id)},
             {"page_ready", primary_context_.page_ready ? "true" : "false"},
             {"renderer_ready", primary_context_.renderer_ready ? "true" : "false"},
             {"runtime_ready", primary_context_.runtime_ready ? "true" : "false"},
             {"load_in_progress", primary_context_.load_in_progress ? "true" : "false"},
             {"browser_closed", primary_context_.browser_closed ? "true" : "false"},
             {"current_url", primary_context_.current_url},
             {"thread", ThreadLabel()}});
        AppendDebugLog(
            std::string("host: pump_until_waiting message=") + timeout_message +
            " elapsed_ms=" +
            std::to_string(
                std::chrono::duration_cast<std::chrono::milliseconds>(now - started_at).count()) +
            " startup_complete=" + (startup_complete_ ? "true" : "false") +
            " browser_available=" + (browser_.get() != nullptr ? "true" : "false") +
            " context_id=" + primary_context_.context_id +
            " browser_id=" + std::to_string(primary_context_.browser_id) +
            " page_ready=" + (primary_context_.page_ready ? "true" : "false") +
            " renderer_ready=" + (primary_context_.renderer_ready ? "true" : "false") +
            " runtime_ready=" + (primary_context_.runtime_ready ? "true" : "false") +
            " load_in_progress=" + (primary_context_.load_in_progress ? "true" : "false") +
            " browser_closed=" + (primary_context_.browser_closed ? "true" : "false") +
            " current_url=" + primary_context_.current_url + " " + ThreadLabel());
        last_wait_log_at = now;
      }

      AegisPumpBrowserHostWindow();
      CefDoMessageLoopWork();
      std::this_thread::sleep_for(kPumpInterval);
    }
  }

  std::vector<std::uint8_t> EncodeJsonEnvelope(MessageKind kind, const std::string& json) {
    return EncodeEnvelope(kind, ParseJsonValue(json, "renderer response is not valid json"));
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
    primary_context_.local_events.push_back(WriteJson(event));
    SyncPrimaryContextToRegistryLocked();
  }

  std::vector<std::string> DrainLocalEvents() {
    std::lock_guard lock(mutex_);
    auto events = std::move(primary_context_.local_events);
    primary_context_.local_events.clear();
    SyncPrimaryContextToRegistryLocked();
    return events;
  }

  void Start() {
    try {
      AppendDebugLog("host: start");
      RequireOwnerThread();
      AegisPlatformInitializeMainApplication(true);
      AegisPlatformConfigureActivation(true, !options_.headless);

      AppendDebugLog("host: cef_load_library begin");
      if (!cef_load_library(paths_.cef_library.string().c_str())) {
        throw std::runtime_error("failed to load Chromium Embedded Framework runtime");
      }
      AppendDebugLog("host: cef_load_library complete");

      CefMainArgs main_args;
      AegisCefBootstrapOptions bootstrap_options;
      bootstrap_options.headless = options_.headless;
      bootstrap_options.external_message_pump = false;
      bootstrap_options.initialize_browser_host_application = !options_.headless;
      bootstrap_options.browser_subprocess_path = paths_.helper_executable.string();
      bootstrap_options.framework_dir_path = paths_.framework_dir.string();
      bootstrap_options.main_bundle_path = paths_.main_bundle_path.string();
      bootstrap_options.resources_dir_path = paths_.resources_dir.string();
      bootstrap_options.locales_dir_path = paths_.locales_dir.string();
      // Always isolate Chromium disk state per runtime instance so serve
      // sessions never collide on the shared default singleton path.
      bootstrap_options.root_cache_path = runtime_session_paths_.instance_dir.string();
      bootstrap_options.cache_path =
          (runtime_session_paths_.instance_dir / "cache").string();
      app_ = new AegisApp(false);
      int subprocess_exit_code = -1;
      std::string initialize_error;
      AppendDebugLog("host: canonical cef bootstrap begin");
      AppendDebugLog("host: cef_execute_process begin");
      const bool initialized = AegisExecuteProcessAndInitialize(
          main_args, bootstrap_options, app_, &subprocess_exit_code, &initialize_error);
      AppendDebugLog("host: cef_execute_process complete exit_code=" +
                     std::to_string(subprocess_exit_code));
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
      ApplyAegisProductionPreferences(CefPreferenceManager::GetGlobalPreferenceManager(),
                                     download_dir_.string());
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
        cef_unload_library();
        cef_initialized_ = false;
      }
      AegisRemoveRuntimeSession(runtime_session_paths_);
      std::lock_guard lock(mutex_);
      startup_error_ = error.what();
      cv_.notify_all();
    }
  }

  void Shutdown(bool shutdown_cef_runtime) {
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
        PumpUntil([this]() { return primary_context_.browser_closed || browser_.get() == nullptr; }, deadline,
                  "timed out waiting for browser shutdown");
      }
      if (shutdown_cef_runtime) {
        CefShutdown();
        cef_unload_library();
      }
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
    CefWindowHandle external_host_view = kNullWindowHandle;
    const bool create_headful_on_owner_thread =
        !options_.headless && AegisUseExternalBrowserHostWindow();
    if (!options_.headless && AegisUseExternalBrowserHostWindow()) {
      RequireOwnerThread();
      external_host_view = AegisCreateBrowserHostView("Aegis", 1280, 800);
      AegisShowBrowserHostWindow();
      AppendDebugLog(std::string("host: prepared_external_browser_host_view handle=") +
                     std::to_string(reinterpret_cast<std::uintptr_t>(external_host_view)) + " " +
                     ThreadLabel());
      AppendTelemetry("prepared_external_browser_host_view",
                      {{"handle",
                        std::to_string(reinterpret_cast<std::uintptr_t>(external_host_view))},
                       {"thread", ThreadLabel()}});
    }

    auto task = [this, external_host_view]() {
      AppendDebugLog(std::string("host: create_browser_on_ui_thread headless=") +
                     (options_.headless ? "true" : "false") +
                     " external_host_view=" +
                     std::to_string(reinterpret_cast<std::uintptr_t>(external_host_view)) + " " +
                     ThreadLabel());
      AppendTelemetry("create_browser_entry",
                      {{"headless", options_.headless ? "true" : "false"},
                       {"external_host_view",
                        std::to_string(reinterpret_cast<std::uintptr_t>(external_host_view))},
                       {"thread", ThreadLabel()}});

      CefBrowserSettings settings;
      settings.windowless_frame_rate = 30;

      client_ = new AegisHostClient(options_.headless,
                                    static_cast<::AegisClientDelegate*>(this), this);
      const auto initial_url = options_.start_url.empty() ? std::string(aegis::kBootstrapUrl)
                                                          : options_.start_url;

      if (options_.headless) {
        CefRequestContextSettings request_context_settings;
        request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
        if (!request_context_.get()) {
          throw std::runtime_error("failed to create request context");
        }
        const bool bootstrap_registered = request_context_->RegisterSchemeHandlerFactory(
            std::string(aegis::kBootstrapScheme),
            std::string(aegis::kBootstrapDomain),
            new BootstrapSchemeHandlerFactory());
        AppendDebugLog(std::string("host: headless request_context bootstrap_handler_registered=") +
                       (bootstrap_registered ? "true" : "false"));
        ApplyAegisProductionPreferences(request_context_, download_dir_.string());
        CefWindowInfo window_info;
        window_info.SetAsWindowless(kNullWindowHandle);
        window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
        auto browser = CefBrowserHost::CreateBrowserSync(
            window_info, client_, initial_url, settings, nullptr, request_context_);
        const bool created = browser.get() != nullptr;
        AppendDebugLog(std::string("host: create_headless_browser_sync result=") +
                       (created ? "true" : "false") + " url=" + initial_url + " " +
                       ThreadLabel());
        AppendTelemetry("create_headless_browser",
                        {{"ok", created ? "true" : "false"},
                         {"url", initial_url},
                         {"thread", ThreadLabel()}});
        if (!created) {
          throw std::runtime_error("failed to create headless browser");
        }
        {
          std::lock_guard lock(mutex_);
          if (!browser_.get()) {
            browser_ = browser;
            request_context_ = browser->GetHost()->GetRequestContext();
            primary_context_.AttachBrowser(browser);
            SyncPrimaryContextToRegistryLocked();
          }
        }
        return;
      }
      CefRequestContextSettings request_context_settings;
      request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
      if (!request_context_.get()) {
        throw std::runtime_error("failed to create request context");
      }
      const bool bootstrap_registered = request_context_->RegisterSchemeHandlerFactory(
          std::string(aegis::kBootstrapScheme),
          std::string(aegis::kBootstrapDomain),
          new BootstrapSchemeHandlerFactory());
      AppendDebugLog(std::string("host: headful request_context bootstrap_handler_registered=") +
                     (bootstrap_registered ? "true" : "false"));
      ApplyAegisProductionPreferences(request_context_, download_dir_.string());
      CefWindowInfo window_info;
      if (AegisUseExternalBrowserHostWindow()) {
        if (external_host_view == kNullWindowHandle) {
          throw std::runtime_error("browser host view is not available");
        }
        window_info.SetAsChild(external_host_view, CefRect(0, 0, 1280, 800));
      } else {
#if defined(__APPLE__)
        window_info.hidden = false;
        CefString(&window_info.window_name) = "Aegis";
#else
        window_info.SetAsPopup(kNullWindowHandle, "Aegis");
#endif
      }
      window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
      auto browser = CefBrowserHost::CreateBrowserSync(
          window_info, client_, initial_url, settings, nullptr, request_context_);
      const bool created = browser.get() != nullptr;
      AppendDebugLog(std::string("host: create_headful_browser_sync result=") +
                     (created ? "true" : "false") + " url=" + initial_url + " " +
                     ThreadLabel());
      AppendTelemetry("create_headful_browser",
                      {{"ok", created ? "true" : "false"},
                       {"url", initial_url},
                       {"thread", ThreadLabel()}});
      if (!created) {
        throw std::runtime_error("failed to create headful browser");
      }
      {
        std::lock_guard lock(mutex_);
        if (!browser_.get()) {
          browser_ = browser;
          request_context_ = browser->GetHost()->GetRequestContext();
          primary_context_.AttachBrowser(browser);
          SyncPrimaryContextToRegistryLocked();
        }
      }
    };

    if (create_headful_on_owner_thread) {
      RequireOwnerThread();
      task();
      return;
    }
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

  void EnsureBrowserAvailable() {
    RequireOwnerThread();
    AppendDebugLog(std::string("host: ensure_browser_available enter ") + ThreadLabel());
    const auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    PumpUntil([this]() { return startup_complete_ || !startup_error_.empty(); }, deadline,
              "timed out waiting for CEF startup");
    PumpUntil([this]() { return browser_.get() != nullptr; }, deadline,
              "timed out waiting for browser availability");
    AppendDebugLog("host: ensure_browser_available complete");
  }

  void EnsureRendererReady() {
    RequireOwnerThread();
    AppendDebugLog("host: ensure_renderer_ready enter");
    EnsureBrowserAvailable();
    const auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    PumpUntil([this]() { return primary_context_.renderer_ready; }, deadline,
              "timed out waiting for ready renderer context");
    AppendDebugLog("host: ensure_renderer_ready complete");
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
    return primary_context_.current_url;
  }

  void NavigateTo(const std::string& url) {
    SetOperationStage("waiting for navigable browser state");
    EnsureBrowserAvailable();
    const auto deadline = std::chrono::steady_clock::now() + kStartupTimeout;
    PumpUntil([this]() { return !primary_context_.load_in_progress; }, deadline,
              "timed out waiting for browser navigation readiness");

    SetOperationStage("preparing browser navigation");
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() != nullptr && primary_context_.current_url == url &&
          (primary_context_.renderer_ready || primary_context_.page_ready)) {
        AppendDebugLog("host: navigate_to skipped_same_url");
        return;
      }
      primary_context_.BeginNavigation(url);
      SyncPrimaryContextToRegistryLocked();
    }
    SetOperationStage("dispatching LoadURL on UI thread");
    RunOnUiThreadSync([this, url]() {
      CEF_REQUIRE_UI_THREAD();
      if (!browser_.get()) {
        throw std::runtime_error("browser is not available");
      }
      browser_->GetMainFrame()->LoadURL(url);
    });
    SetOperationStage("waiting for navigation start");
    PumpUntil([this, &url]() {
      return primary_context_.current_url == url || primary_context_.current_url == url + "/" ||
             primary_context_.current_url.rfind(url + "/", 0) == 0;
    }, deadline, "timed out waiting for navigation start");
  }

  void EnsureRuntimeReady() {
    {
      std::lock_guard lock(mutex_);
      if (browser_.get() != nullptr && primary_context_.runtime_ready) {
        return;
      }
    }
    SetOperationStage("waiting for browser availability");
    EnsureBrowserAvailable();
    SetOperationStage("waiting for renderer context");
    EnsureRendererReady();
    VerifyRuntimeReady();
  }

  void VerifyRuntimeReady() {
    {
      std::lock_guard lock(mutex_);
      if (primary_context_.runtime_ready) {
        return;
      }
    }

    SetOperationStage("verifying renderer runtime api");
    const auto probe = InvokeRendererAssumingReady(
        aegis::kOpEvalJs,
        "(() => !!(window.__aegis && typeof window.__aegis.snapshot === 'function' && "
        "typeof window.__aegis.drainEvents === 'function' && "
        "typeof window.__aegis.currentPageState === 'function' && "
        "typeof window.__aegis.pageResearchData === 'function' && "
        "typeof window.__aegis.drag === 'function' && "
        "typeof window.__aegis.geometry === 'function'))()");
    if (TrimAscii(probe) != "true") {
      InvalidateRuntime("renderer runtime api probe failed");
      throw std::runtime_error(
          "renderer runtime api is unavailable or missing required research helpers");
    }
    {
      std::lock_guard lock(mutex_);
      primary_context_.runtime_ready = true;
      SyncPrimaryContextToRegistryLocked();
      cv_.notify_all();
    }
  }

  bool TryApplyPendingStorageInjection() {
    std::optional<std::string> payload;
    {
      std::lock_guard lock(mutex_);
      const auto scheme = UrlScheme(primary_context_.current_url);
      if (!primary_context_.pending_storage_injection_payload.has_value() || !browser_.get() ||
          !primary_context_.renderer_ready || !primary_context_.runtime_ready ||
          !scheme.has_value() || aegis::IsBootstrapUrl(primary_context_.current_url) ||
          primary_context_.current_url == "about:blank") {
        return false;
      }
      payload = primary_context_.pending_storage_injection_payload;
    }

    AppendDebugLog("host: applying_pending_storage_injection");
    SetOperationStage("injecting deferred storage");
    InvokeRendererReady(aegis::kOpInjectStorage, *payload);
    {
      std::lock_guard lock(mutex_);
      if (primary_context_.pending_storage_injection_payload == payload) {
        primary_context_.pending_storage_injection_payload.reset();
      }
      SyncPrimaryContextToRegistryLocked();
    }
    AppendDebugLog("host: applied_pending_storage_injection");
    return true;
  }

  std::string InvokeRendererReady(const std::string& operation, const std::string& body) {
    EnsureRuntimeReady();
    if (operation != aegis::kOpInjectStorage) {
      TryApplyPendingStorageInjection();
    }
    return InvokeRendererAssumingReady(operation, body);
  }

  std::string InvokeRendererAssumingReady(const std::string& operation,
                                          const std::string& body) {
    try {
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
        return primary_context_.renderer_replies.contains(request_id) || !startup_error_.empty();
      }, deadline, "timed out waiting for renderer response");

      RendererReply reply;
      {
        std::lock_guard lock(mutex_);
        reply = std::move(primary_context_.renderer_replies.at(request_id));
        primary_context_.renderer_replies.erase(request_id);
      }
      if (!reply.ok) {
        InvalidateRuntime(reply.body);
        throw std::runtime_error(reply.body);
      }
      {
        std::lock_guard lock(mutex_);
        primary_context_.runtime_ready = true;
        SyncPrimaryContextToRegistryLocked();
        cv_.notify_all();
      }
      AppendDebugLog("host: invoke_renderer complete " + operation);
      return reply.body;
    } catch (const std::exception& error) {
      InvalidateRuntime(error.what());
      throw;
    }
  }

  std::string InvokeRenderer(const std::string& operation, const std::string& body) {
    return InvokeRendererReady(operation, body);
  }

  void InvalidateRuntime(const std::string& reason) {
    std::lock_guard lock(mutex_);
    primary_context_.InvalidateRuntime(
        reason.find("timed out waiting for renderer response") != std::string::npos ||
        reason.find("browser is not available") != std::string::npos ||
        reason.find("main frame is not available") != std::string::npos);
    SyncPrimaryContextToRegistryLocked();
    cv_.notify_all();
  }

  void CompleteRendererRequest(int request_id, bool ok, std::string body) {
    std::lock_guard lock(mutex_);
    primary_context_.renderer_replies[request_id] = RendererReply{
        .ok = ok,
        .body = std::move(body),
    };
    SyncPrimaryContextToRegistryLocked();
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
    auto existing = std::find_if(primary_context_.cookie_jar.begin(),
                                 primary_context_.cookie_jar.end(), matches);
    if (remove_cookie) {
      if (existing != primary_context_.cookie_jar.end()) {
        primary_context_.cookie_jar.erase(existing);
      }
      SyncPrimaryContextToRegistryLocked();
      return;
    }
    if (existing != primary_context_.cookie_jar.end()) {
      *existing = std::move(cookie);
      SyncPrimaryContextToRegistryLocked();
      return;
    }
    primary_context_.cookie_jar.push_back(std::move(cookie));
    SyncPrimaryContextToRegistryLocked();
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
            .same_site = cookie_value->HasKey("same_site")
                             ? std::optional<std::string>(
                                   cookie_value->GetString("same_site").ToString())
                             : std::nullopt,
        };
        jar.push_back(std::move(cookie));
      }
    }
    std::lock_guard lock(mutex_);
    primary_context_.cookie_jar = std::move(jar);
    SyncPrimaryContextToRegistryLocked();
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
      } else if (CaseEqualAscii(key, "samesite")) {
        if (CaseEqualAscii(value, "none")) {
          cookie.same_site = "none";
        } else if (CaseEqualAscii(value, "lax")) {
          cookie.same_site = "lax";
        } else if (CaseEqualAscii(value, "strict")) {
          cookie.same_site = "strict";
        }
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
      url = primary_context_.current_url;
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
    primary_context_.network_overrides = std::move(overrides);
    SyncPrimaryContextToRegistryLocked();
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
      if (cookie_value->HasKey("same_site")) {
        cookie.same_site =
            ParseCookieSameSite(cookie_value->GetString("same_site").ToString());
      }

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
    SnapshotCookies();
  }

  CefRefPtr<CefListValue> SnapshotCookies() {
    auto list = CefListValue::Create();
    auto manager = request_context_ ? request_context_->GetCookieManager(nullptr) : nullptr;
    std::vector<ManagedCookie> managed;
    if (manager.get()) {
      std::vector<CefCookie> cookies;
      auto done = CefWaitableEvent::CreateWaitableEvent(true, false);
      manager->VisitAllCookies(new CookieCollector(&cookies, done));
      done->Wait();
      managed.reserve(cookies.size());
      for (const auto& cookie : cookies) {
        managed.push_back(ManagedCookieFromCef(cookie));
      }
      {
        std::lock_guard lock(mutex_);
        primary_context_.cookie_jar = managed;
        SyncPrimaryContextToRegistryLocked();
      }
    } else {
      std::lock_guard lock(mutex_);
      managed = primary_context_.cookie_jar;
    }

    for (std::size_t index = 0; index < managed.size(); ++index) {
      const auto& cookie = managed[index];
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
      if (cookie.same_site.has_value()) {
        entry->SetString("same_site", *cookie.same_site);
      }
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  CefRefPtr<CefListValue> SnapshotNetworkOverrides() {
    std::lock_guard lock(mutex_);
    auto list = CefListValue::Create();
    for (std::size_t index = 0; index < primary_context_.network_overrides.size(); ++index) {
      auto entry = CefDictionaryValue::Create();
      entry->SetString("header", primary_context_.network_overrides[index].first);
      entry->SetString("value", primary_context_.network_overrides[index].second);
      list->SetDictionary(static_cast<int>(index), entry);
    }
    return list;
  }

  const BrowserOptions options_;
  const std::filesystem::path download_dir_;
  const HostPaths paths_;
  const AegisRuntimeSessionPaths runtime_session_paths_;
  const std::thread::id owner_thread_id_;
  const bool manage_cef_lifecycle_;
  const bool counted_shared_lifecycle_;

  CefRefPtr<AegisApp> app_;
  CefRefPtr<AegisClient> client_;
  CefRefPtr<CefRegistration> devtools_registration_;

  mutable std::mutex mutex_;
  std::condition_variable cv_;
  bool cef_initialized_ = false;
  bool startup_complete_ = false;
  bool devtools_network_enabled_ = false;
  std::string startup_error_;
  int next_request_id_ = 1;
  BrowserContextState primary_context_;
  std::map<int, BrowserContextState> browser_contexts_;
  int active_browser_id_ = 0;
  std::string current_operation_name_;
  std::string current_operation_stage_;
  std::chrono::steady_clock::time_point current_operation_started_at_ =
      std::chrono::steady_clock::now();
  std::atomic<bool> cancel_requested_{false};

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


void AegisDevToolsObserver::OnDevToolsEvent(CefRefPtr<CefBrowser> browser,
                                            const CefString& method,
                                            const void* params,
                                            size_t params_size) {
  if (host_ != nullptr) {
    host_->HandleDevToolsEvent(browser, method, params, params_size);
  }
}

void AegisDevToolsObserver::OnDevToolsAgentAttached(CefRefPtr<CefBrowser> browser) {
  if (host_ != nullptr) {
    host_->HandleDevToolsAgentAttached(browser);
  }
}

void AegisDevToolsObserver::OnDevToolsAgentDetached(CefRefPtr<CefBrowser> browser) {
  if (host_ != nullptr) {
    host_->HandleDevToolsAgentDetached(browser);
  }
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

AegisHostStatus EnsureRuntime(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::EnsureRuntime);
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

AegisHostStatus SnapshotHostState(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::SnapshotHostState);
}

AegisHostStatus ActivateBrowser(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::ActivateBrowser);
}

AegisHostStatus Pump(
    AegisHostHandle ctx,
    const std::uint8_t* input_ptr,
    std::size_t input_len,
    AegisHostBuffer* output) {
  return Dispatch(ctx, input_ptr, input_len, output, &CefHost::Pump);
}

void RequestCancel(AegisHostHandle ctx) {
  if (ctx == nullptr) {
    return;
  }
  auto* host = static_cast<CefHost*>(ctx);
  host->RequestCancel();
}

void FreeBuffer(AegisHostHandle, AegisHostBuffer buffer) {
  delete[] buffer.ptr;
}

}  // namespace

AegisHostFunctionTable ExportFunctionTable() {
  return AegisHostFunctionTable{
      .ensure_runtime = EnsureRuntime,
      .eval_js = EvalJs,
      .send_batch = SendBatch,
      .snapshot_dom = SnapshotDom,
      .inject_session = InjectSession,
      .snapshot_session = SnapshotSession,
      .drain_events = DrainEvents,
      .navigate = Navigate,
      .snapshot_host_state = SnapshotHostState,
      .activate_browser = ActivateBrowser,
      .pump = Pump,
      .request_cancel = RequestCancel,
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

    if (operation != EmbeddedHostOperation::EnsureRuntime) {
      const auto runtime_path = std::filesystem::current_path() / "assets" / "js" / "aegis_runtime.js";
      if (std::filesystem::exists(runtime_path)) {
        std::ifstream runtime_input(runtime_path, std::ios::binary);
        if (runtime_input.is_open()) {
          const std::string script((std::istreambuf_iterator<char>(runtime_input)),
                                   std::istreambuf_iterator<char>());
          auto runtime_value = CefValue::Create();
          runtime_value->SetString(script);
          host.EnsureRuntime(EncodeEnvelope(MessageKind::EnsureRuntime, runtime_value));
        }
      }
    }

    switch (operation) {
      case EmbeddedHostOperation::EnsureRuntime:
        *response = host.EnsureRuntime(request);
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
      case EmbeddedHostOperation::SnapshotHostState:
        *response = host.SnapshotHostState(request);
        break;
      case EmbeddedHostOperation::ActivateBrowser:
        *response = host.ActivateBrowser(request);
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
  bool manage_cef_lifecycle = false;
  try {
    {
      std::lock_guard lock(aegis::g_shared_host_lifecycle_mutex);
      manage_cef_lifecycle = aegis::g_shared_host_count == 0;
      ++aegis::g_shared_host_count;
    }
    auto host = std::make_unique<aegis::AegisCefHost>(
        aegis::ParseBrowserOptions(aegis::CopyInput(input_ptr, input_len)),
        manage_cef_lifecycle,
        true);
    host->WaitForReady();
    return host.release();
  } catch (const std::exception& ex) {
    std::lock_guard lock(aegis::g_shared_host_lifecycle_mutex);
    if (aegis::g_shared_host_count > 0) {
      --aegis::g_shared_host_count;
    }
    aegis::g_last_host_error = ex.what();
    return nullptr;
  } catch (...) {
    std::lock_guard lock(aegis::g_shared_host_lifecycle_mutex);
    if (aegis::g_shared_host_count > 0) {
      --aegis::g_shared_host_count;
    }
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
