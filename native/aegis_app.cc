#include "aegis_app.h"

#include <cctype>
#include <cstdlib>
#include <fstream>
#include <string>

#include "aegis_client.h"
#include "aegis_messages.h"
#include "aegis_native_mac.h"
#include "aegis_state_paths.h"
#include "build-xcode/generated/aegis_runtime_script.h"
#if defined(AEGIS_STANDALONE_APP)
#include "aegis_native_mac.h"
#endif
#include "include/cef_browser.h"
#include "include/cef_command_line.h"
#include "include/cef_parser.h"
#include "include/cef_preference.h"
#include "include/cef_process_message.h"
#include "include/cef_v8.h"
#include "include/wrapper/cef_helpers.h"

namespace {

constexpr char kBootstrapUrl[] =
    "data:text/html,%3C!doctype%20html%3E%3Chtml%3E%3Chead%3E%3Cmeta%20charset%3D%22utf-8%22%3E%3C%2Fhead%3E%3Cbody%3E%3C%2Fbody%3E%3C%2Fhtml%3E";

void AppendDebugLog(const std::string& message) {
  std::string path;
  if (const char* env_path = std::getenv("AEGIS_DEBUG_LOG");
      env_path != nullptr && *env_path != '\0') {
    path = env_path;
  } else if (auto command_line = CefCommandLine::GetGlobalCommandLine(); command_line.get()) {
    path = command_line->GetSwitchValue("aegis-debug-log").ToString();
  }
  if (path.empty()) {
    return;
  }
  std::ofstream output(path, std::ios::app);
  if (!output.is_open()) {
    return;
  }
  output << message << '\n';
}

bool IsHeadless(CefRefPtr<CefCommandLine> command_line) {
  return command_line->HasSwitch("headless") ||
         command_line->GetSwitchValue("mode") == "headless";
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
    AppendDebugLog(std::string("app: failed_to_set_preference ") + name + " " +
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

#if defined(AEGIS_STANDALONE_APP)
std::string RequestedStartupUrl(CefRefPtr<CefCommandLine> command_line) {
  return command_line->GetSwitchValue("url").ToString();
}
#endif

std::string QuoteForJavaScript(const std::string& input) {
  std::string out;
  out.reserve(input.size() + 8);
  out.push_back('"');
  for (const char ch : input) {
    switch (ch) {
      case '\\':
        out += "\\\\";
        break;
      case '"':
        out += "\\\"";
        break;
      case '\n':
        out += "\\n";
        break;
      case '\r':
        out += "\\r";
        break;
      case '\t':
        out += "\\t";
        break;
      default:
        out.push_back(ch);
        break;
    }
  }
  out.push_back('"');
  return out;
}

std::string QuoteForJson(const std::string& input) {
  std::string out;
  out.reserve(input.size() + 8);
  out.push_back('"');
  for (const unsigned char ch : input) {
    switch (ch) {
      case '\\':
        out += "\\\\";
        break;
      case '"':
        out += "\\\"";
        break;
      case '\b':
        out += "\\b";
        break;
      case '\f':
        out += "\\f";
        break;
      case '\n':
        out += "\\n";
        break;
      case '\r':
        out += "\\r";
        break;
      case '\t':
        out += "\\t";
        break;
      default:
        if (ch < 0x20) {
          constexpr char kHex[] = "0123456789abcdef";
          out += "\\u00";
          out.push_back(kHex[(ch >> 4) & 0x0f]);
          out.push_back(kHex[ch & 0x0f]);
        } else {
          out.push_back(static_cast<char>(ch));
        }
        break;
    }
  }
  out.push_back('"');
  return out;
}

bool CommandTargetArgument(CefRefPtr<CefDictionaryValue> command,
                           std::string* argument,
                           std::string* error) {
  if (command->HasKey("match")) {
    auto matcher = command->GetDictionary("match");
    if (!matcher.get()) {
      *error = "command match target must be an object";
      return false;
    }
    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(matcher->Copy(false));
    *argument = CefWriteJSON(wrapped, JSON_WRITER_DEFAULT).ToString();
    return true;
  }
  if (command->HasKey("id")) {
    *argument = std::to_string(command->GetInt("id"));
    return true;
  }
  *error = "command target must include id or match";
  return false;
}

bool OptionalNestedTargetArgument(CefRefPtr<CefDictionaryValue> command,
                                  const char* key,
                                  std::string* argument,
                                  std::string* error) {
  if (!command->HasKey(key)) {
    *argument = "null";
    return true;
  }
  auto wrapper = command->GetDictionary(key);
  if (!wrapper.get()) {
    *error = "command target wrapper must be an object";
    return false;
  }
  if (wrapper->HasKey("match")) {
    auto matcher = wrapper->GetDictionary("match");
    if (!matcher.get()) {
      *error = "command match target must be an object";
      return false;
    }
    auto wrapped = CefValue::Create();
    wrapped->SetDictionary(matcher->Copy(false));
    *argument = CefWriteJSON(wrapped, JSON_WRITER_DEFAULT).ToString();
    return true;
  }
  if (wrapper->HasKey("id")) {
    *argument = std::to_string(wrapper->GetInt("id"));
    return true;
  }
  *argument = "null";
  return true;
}

std::string NormalizeEvalCode(std::string code) {
  auto trim = [](std::string* value) {
    std::size_t start = 0;
    while (start < value->size() &&
           std::isspace(static_cast<unsigned char>((*value)[start])) != 0) {
      ++start;
    }
    std::size_t end = value->size();
    while (end > start &&
           std::isspace(static_cast<unsigned char>((*value)[end - 1])) != 0) {
      --end;
    }
    *value = value->substr(start, end - start);
  };

  trim(&code);
  constexpr char kReturnPrefix[] = "return ";
  if (code.rfind(kReturnPrefix, 0) == 0) {
    code.erase(0, sizeof(kReturnPrefix) - 1);
    trim(&code);
    if (!code.empty() && code.back() == ';') {
      code.pop_back();
      trim(&code);
    }
  }
  return code;
}

bool EvalToString(CefRefPtr<CefFrame> frame,
                  const std::string& code,
                  std::string* value,
                  std::string* error) {
  auto context = frame->GetV8Context();
  if (!context.get()) {
    *error = "v8 context unavailable";
    return false;
  }
  if (!context->Enter()) {
    *error = "failed to enter v8 context";
    return false;
  }

  CefRefPtr<CefV8Value> result;
  CefRefPtr<CefV8Exception> exception;
  const bool ok = context->Eval(code, frame->GetURL(), 0, result, exception);
  context->Exit();

  if (!ok) {
    *error = exception.get() ? exception->GetMessage().ToString()
                             : std::string("javascript evaluation failed");
    return false;
  }

  if (!result.get()) {
    value->clear();
    return true;
  }
  if (result->IsString()) {
    *value = result->GetStringValue().ToString();
    return true;
  }
  if (result->IsBool()) {
    *value = result->GetBoolValue() ? "true" : "false";
    return true;
  }
  if (result->IsInt() || result->IsUInt()) {
    *value = std::to_string(result->GetIntValue());
    return true;
  }
  if (result->IsDouble()) {
    *value = std::to_string(result->GetDoubleValue());
    return true;
  }
  value->clear();
  return true;
}

bool EvalToJson(CefRefPtr<CefFrame> frame,
                const std::string& code,
                std::string* value,
                std::string* error) {
  auto context = frame->GetV8Context();
  if (!context.get()) {
    *error = "v8 context unavailable";
    return false;
  }
  if (!context->Enter()) {
    *error = "failed to enter v8 context";
    return false;
  }

  CefRefPtr<CefV8Value> result;
  CefRefPtr<CefV8Exception> exception;
  const bool ok = context->Eval(code, frame->GetURL(), 0, result, exception);
  if (!ok) {
    context->Exit();
    *error = exception.get() ? exception->GetMessage().ToString()
                             : std::string("javascript evaluation failed");
    return false;
  }

  if (!result.get() || result->IsUndefined() || result->IsNull()) {
    *value = "null";
    context->Exit();
    return true;
  }
  if (result->IsString()) {
    *value = QuoteForJson(result->GetStringValue().ToString());
    context->Exit();
    return true;
  }
  if (result->IsBool()) {
    *value = result->GetBoolValue() ? "true" : "false";
    context->Exit();
    return true;
  }
  if (result->IsInt() || result->IsUInt()) {
    *value = std::to_string(result->GetIntValue());
    context->Exit();
    return true;
  }
  if (result->IsDouble()) {
    *value = std::to_string(result->GetDoubleValue());
    context->Exit();
    return true;
  }

  auto global = context->GetGlobal();
  if (!global.get()) {
    context->Exit();
    *error = "v8 global unavailable";
    return false;
  }

  const CefString temp_name("__aegis_eval_value");
  global->SetValue(temp_name, result, V8_PROPERTY_ATTRIBUTE_NONE);

  CefRefPtr<CefV8Value> json_result;
  CefRefPtr<CefV8Exception> json_exception;
  const bool json_ok = context->Eval("JSON.stringify(globalThis.__aegis_eval_value)",
                                     frame->GetURL(), 0, json_result, json_exception);
  global->DeleteValue(temp_name);
  context->Exit();

  if (!json_ok) {
    *error = json_exception.get() ? json_exception->GetMessage().ToString()
                                  : std::string("failed to serialize javascript value");
    return false;
  }

  if (!json_result.get() || json_result->IsUndefined() || json_result->IsNull()) {
    *value = "null";
    return true;
  }
  if (!json_result->IsString()) {
    *error = "serialized javascript value is not a string";
    return false;
  }

  *value = json_result->GetStringValue().ToString();
  return true;
}

void SendRendererReply(CefRefPtr<CefFrame> frame,
                       int request_id,
                       bool ok,
                       const std::string& body) {
  auto reply = CefProcessMessage::Create(aegis::kAegisResponseMessage);
  auto args = reply->GetArgumentList();
  args->SetInt(0, request_id);
  args->SetBool(1, ok);
  args->SetString(2, body);
  frame->SendProcessMessage(PID_BROWSER, reply);
}

bool DispatchRendererOperation(const std::string& op,
                               const std::string& body,
                               CefRefPtr<CefFrame> frame,
                               std::string* response,
                               std::string* error) {
  if (op == aegis::kOpInstallRuntime) {
    if (!EvalToString(frame, body, response, error)) {
      return false;
    }
    *response = "{}";
    return true;
  }

  if (op == aegis::kOpEvalJs) {
    return EvalToString(frame, body, response, error);
  }

  if (op == aegis::kOpSnapshotDom) {
    return EvalToString(frame,
                        "JSON.stringify(window.__aegis ? window.__aegis.snapshot() : {nodes:[]})",
                        response, error);
  }

  if (op == aegis::kOpDrainEvents) {
    return EvalToString(
        frame,
        "JSON.stringify({events: window.__aegis ? window.__aegis.drainEvents() : []})",
        response, error);
  }

  if (op == aegis::kOpSnapshotStorage) {
    return EvalToString(
        frame,
        R"((() => {
          const readStorage = (getter) => {
            try {
              const store = getter();
              return Object.fromEntries(Object.entries(store));
            } catch (_) {
              return {};
            }
          };
          return JSON.stringify({
            cookies: [],
            local_storage: readStorage(() => localStorage),
            session_storage: readStorage(() => sessionStorage),
            network_overrides: []
          });
        })())",
        response, error);
  }

  if (op == aegis::kOpInjectStorage) {
    const auto quoted = QuoteForJavaScript(body);
    return EvalToString(
        frame,
        "(() => { const s = JSON.parse(" + quoted +
            "); const writeStorage = (getter, values) => { try { const store = getter(); for (const [k,v] of Object.entries(values || {})) store.setItem(k, v); } catch (_) {} };"
            " writeStorage(() => localStorage, s.local_storage);"
            " writeStorage(() => sessionStorage, s.session_storage);"
            " return '{}'; })()",
        response, error);
  }

  if (op == aegis::kOpSendBatch) {
    CefRefPtr<CefValue> parsed = CefParseJSON(body, JSON_PARSER_RFC);
    if (!parsed.get() || parsed->GetType() != VTYPE_DICTIONARY) {
      *error = "batch request is not valid json";
      return false;
    }

    auto request = parsed->GetDictionary();
    auto commands = request->HasKey("commands") ? request->GetList("commands") : CefListValue::Create();
    const int batch_id = request->HasKey("batch_id") ? request->GetInt("batch_id") : 0;
    bool requires_snapshot = false;

    std::string results_json = "[";
    if (commands.get()) {
      for (size_t index = 0; index < commands->GetSize(); ++index) {
        auto command = commands->GetDictionary(static_cast<int>(index));
        if (!command.get()) {
          if (index != 0) {
            results_json += ",";
          }
          results_json += R"({"ok":false,"error":"invalid command payload"})";
          continue;
        }

        const auto type = command->GetString("type").ToString();
        if (type != "eval" && type != "scroll" && type != "press_key") {
          requires_snapshot = true;
        }
        std::string command_result;

        if (type == "eval") {
          const auto code = NormalizeEvalCode(command->GetString("code").ToString());
          std::string value_json;
          std::string command_error;
          if (EvalToJson(frame, code, &value_json, &command_error)) {
            command_result = "{\"ok\":true,\"value\":" + value_json + "}";
          } else {
            command_result =
                "{\"ok\":false,\"error\":" + QuoteForJson(command_error) + "}";
          }
        } else if (type == "click") {
          std::string target_argument;
          if (!CommandTargetArgument(command, &target_argument, error)) {
            return false;
          }
          const auto wrapped =
              "(() => { try { return JSON.stringify({ok:true,value:(window.__aegis ? window.__aegis.click(" +
              target_argument +
              ") : null)}); } catch (error) { return JSON.stringify({ok:false,error:String(error && error.message ? error.message : error)}); } })()";
          if (!EvalToString(frame, wrapped, &command_result, error)) {
            return false;
          }
        } else if (type == "set_value") {
          std::string target_argument;
          if (!CommandTargetArgument(command, &target_argument, error)) {
            return false;
          }
          const auto value_json = QuoteForJavaScript(command->GetString("value").ToString());
          const auto wrapped =
              "(() => { try { return JSON.stringify({ok:true,value:(window.__aegis ? window.__aegis.setValue(" +
              target_argument + "," + value_json +
              ") : null)}); } catch (error) { return JSON.stringify({ok:false,error:String(error && error.message ? error.message : error)}); } })()";
          if (!EvalToString(frame, wrapped, &command_result, error)) {
            return false;
          }
        } else if (type == "hover") {
          std::string target_argument;
          if (!CommandTargetArgument(command, &target_argument, error)) {
            return false;
          }
          const auto wrapped =
              "(() => { try { return JSON.stringify({ok:true,value:(window.__aegis ? window.__aegis.hover(" +
              target_argument +
              ") : null)}); } catch (error) { return JSON.stringify({ok:false,error:String(error && error.message ? error.message : error)}); } })()";
          if (!EvalToString(frame, wrapped, &command_result, error)) {
            return false;
          }
        } else if (type == "press_key") {
          std::string target_argument;
          if (!OptionalNestedTargetArgument(command, "target", &target_argument, error)) {
            return false;
          }
          const auto key_json = QuoteForJavaScript(command->GetString("key").ToString());
          const auto code_json = command->HasKey("code")
                                     ? QuoteForJavaScript(command->GetString("code").ToString())
                                     : "null";
          const auto alt_key = command->GetBool("alt_key") ? "true" : "false";
          const auto ctrl_key = command->GetBool("ctrl_key") ? "true" : "false";
          const auto meta_key = command->GetBool("meta_key") ? "true" : "false";
          const auto shift_key = command->GetBool("shift_key") ? "true" : "false";
          const auto wrapped =
              "(() => { try { return JSON.stringify({ok:true,value:(window.__aegis ? window.__aegis.pressKey(" +
              target_argument + "," + key_json +
              ",{code:" + code_json +
              ",altKey:" + alt_key +
              ",ctrlKey:" + ctrl_key +
              ",metaKey:" + meta_key +
              ",shiftKey:" + shift_key +
              "}) : null)}); } catch (error) { return JSON.stringify({ok:false,error:String(error && error.message ? error.message : error)}); } })()";
          if (!EvalToString(frame, wrapped, &command_result, error)) {
            return false;
          }
        } else if (type == "scroll") {
          const auto x = std::to_string(command->GetInt("x"));
          const auto y = std::to_string(command->GetInt("y"));
          const auto wrapped =
              "(() => { try { return JSON.stringify({ok:true,value:(window.__aegis ? window.__aegis.scrollToPosition(" +
              x + "," + y +
              ") : null)}); } catch (error) { return JSON.stringify({ok:false,error:String(error && error.message ? error.message : error)}); } })()";
          if (!EvalToString(frame, wrapped, &command_result, error)) {
            return false;
          }
        } else {
          command_result = "{\"ok\":false,\"error\":\"unsupported command " + type + "\"}";
        }

        if (index != 0) {
          results_json += ",";
        }
        results_json += command_result.empty() ? "null" : command_result;
      }
    }
    results_json += "]";

    std::string snapshot_json = "null";
    if (requires_snapshot) {
      if (!EvalToString(frame,
                        "JSON.stringify(window.__aegis ? window.__aegis.snapshot() : {nodes:[]})",
                        &snapshot_json, error)) {
        return false;
      }
    }

    std::string events_wrapper_json;
    if (!EvalToString(frame,
                      "JSON.stringify({events: window.__aegis ? window.__aegis.drainEvents() : []})",
                      &events_wrapper_json, error)) {
      return false;
    }

    auto events_value = CefParseJSON(events_wrapper_json, JSON_PARSER_RFC);
    if (!events_value.get() || events_value->GetType() != VTYPE_DICTIONARY) {
      *error = "batch events response is not valid json";
      return false;
    }
    auto events_dict = events_value->GetDictionary();
    auto events_value_wrapper = CefValue::Create();
    events_value_wrapper->SetList(events_dict->GetList("events"));
    const auto events_json = CefWriteJSON(events_value_wrapper, JSON_WRITER_DEFAULT).ToString();

    *response = "{\"batch_id\":" + std::to_string(batch_id) +
                ",\"results\":" + results_json +
                ",\"snapshot\":" + snapshot_json +
                ",\"events\":" + events_json + "}";
    return true;
  }

  *error = "unsupported renderer operation";
  return false;
}

}  // namespace

AegisApp::AegisApp(bool launch_browser_on_context_initialized,
                   std::string startup_url)
    : launch_browser_on_context_initialized_(launch_browser_on_context_initialized),
      startup_url_(startup_url.empty() ? kBootstrapUrl : std::move(startup_url)),
      pending_startup_url_(startup_url_) {
#if defined(AEGIS_STANDALONE_APP)
  runtime_session_paths_ = AegisCreateRuntimeSessionPaths("app");
#endif
}

void AegisApp::OnBeforeCommandLineProcessing(
    const CefString& process_type,
    CefRefPtr<CefCommandLine> command_line) {
  if (const char* debug_log = std::getenv("AEGIS_DEBUG_LOG");
      debug_log != nullptr && *debug_log != '\0' &&
      !command_line->HasSwitch("aegis-debug-log")) {
    command_line->AppendSwitchWithValue("aegis-debug-log", debug_log);
  }

  if (!process_type.empty()) {
    return;
  }

  command_line->AppendSwitch("deny-permission-prompts");
  command_line->AppendSwitch("disable-notifications");
  command_line->AppendSwitch("disable-background-networking");
  command_line->AppendSwitch("disable-geolocation");
  command_line->AppendSwitch("disable-search-geolocation-disclosure");
  command_line->AppendSwitch("no-default-browser-check");
  command_line->AppendSwitch("no-first-run");
  command_line->AppendSwitch("disable-sync");
  command_line->AppendSwitchWithValue("disable-features",
                                      "LocationProviderManager,NewMacNotificationAPI");

  if (IsHeadless(command_line)) {
    command_line->AppendSwitch("disable-gpu");
    command_line->AppendSwitch("disable-gpu-compositing");
  }
}

void AegisApp::OnBeforeChildProcessLaunch(
    CefRefPtr<CefCommandLine> command_line) {
  if (const char* debug_log = std::getenv("AEGIS_DEBUG_LOG");
      debug_log != nullptr && *debug_log != '\0' &&
      !command_line->HasSwitch("aegis-debug-log")) {
    command_line->AppendSwitchWithValue("aegis-debug-log", debug_log);
  }
}

bool AegisApp::OnAlreadyRunningAppRelaunch(
    CefRefPtr<CefCommandLine> command_line,
    const CefString&) {
  CEF_REQUIRE_UI_THREAD();
#if !defined(AEGIS_STANDALONE_APP)
  (void)command_line;
  return false;
#else
  pending_startup_url_ = RequestedStartupUrl(command_line);
  if (pending_startup_url_.empty()) {
    pending_startup_url_ = startup_url_;
  }
  AppendDebugLog("app: on_already_running_app_relaunch url=" + pending_startup_url_);

  if (primary_browser_) {
    AegisShowBrowserHostWindow();
    if (!pending_startup_url_.empty()) {
      primary_browser_->GetMainFrame()->LoadURL(pending_startup_url_);
    }
    return true;
  }

  if (launch_browser_on_context_initialized_) {
    CreateHeadfulBrowser(pending_startup_url_.empty() ? startup_url_
                                                      : pending_startup_url_);
    return true;
  }

  return false;
#endif
}

void AegisApp::OnScheduleMessagePumpWork(int64_t delay_ms) {
  AegisScheduleCefMessagePumpWork(delay_ms);
}

void AegisApp::CreateHeadfulBrowser(const std::string& url) {
#if !defined(AEGIS_STANDALONE_APP)
  (void)url;
#else
  if (!request_context_) {
    CefRequestContextSettings request_context_settings;
    request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
  }
  if (!request_context_) {
    AppendDebugLog("app: failed to create request context");
    return;
  }
  ApplyAegisProductionPreferences(request_context_);

  CefBrowserSettings settings;

  CefRefPtr<AegisClient> client(new AegisClient(false, this));
  CefWindowInfo window_info;
  window_info.SetAsChild(AegisCreateBrowserHostView("Aegis", 1280, 800),
                         CefRect(0, 0, 1280, 800));
  window_info.runtime_style = CEF_RUNTIME_STYLE_ALLOY;

  if (!CefBrowserHost::CreateBrowser(window_info, client, url, settings, nullptr,
                                     request_context_)) {
    AppendDebugLog("app: failed to create headful browser");
  }
#endif
}

void AegisApp::OnContextInitialized() {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog("app: on_context_initialized");
  ApplyAegisProductionPreferences(CefPreferenceManager::GetGlobalPreferenceManager());
  if (!launch_browser_on_context_initialized_) {
    return;
  }

  auto command_line = CefCommandLine::GetGlobalCommandLine();
  const bool headless = IsHeadless(command_line);
  const auto url =
      pending_startup_url_.empty() ? startup_url_ : pending_startup_url_;
  AppendDebugLog("app: on_context_initialized url=" + url);

  CefBrowserSettings settings;

  CefRefPtr<AegisClient> client(new AegisClient(headless, this));

  if (headless) {
    settings.windowless_frame_rate = 30;
    if (!request_context_) {
      CefRequestContextSettings request_context_settings;
      request_context_ = CefRequestContext::CreateContext(request_context_settings, nullptr);
    }
    if (!request_context_) {
      AppendDebugLog("app: failed to create headless request context");
      return;
    }
    ApplyAegisProductionPreferences(request_context_);
    CefWindowInfo window_info;
    window_info.SetAsWindowless(kNullWindowHandle);
    CefBrowserHost::CreateBrowser(window_info, client, url, settings, nullptr,
                                  request_context_);
    return;
  }
  CreateHeadfulBrowser(url);
}

void AegisApp::OnContextCreated(CefRefPtr<CefBrowser>,
                                CefRefPtr<CefFrame> frame,
                                CefRefPtr<CefV8Context> context) {
  CEF_REQUIRE_RENDERER_THREAD();
  AppendDebugLog("app: on_context_created");
  if (!frame.get() || !frame->IsMain() || !context.get()) {
    return;
  }

  if (!context->Enter()) {
    AppendDebugLog("app: on_context_created failed_enter_context");
    return;
  }

  CefRefPtr<CefV8Value> result;
  CefRefPtr<CefV8Exception> exception;
  const bool installed =
      context->Eval(kAegisRuntimeScript, frame->GetURL(), 0, result, exception);
  context->Exit();
  if (!installed) {
    AppendDebugLog(std::string("app: on_context_created runtime_install_failed ") +
                   (exception.get() ? exception->GetMessage().ToString()
                                    : std::string("unknown")));
    return;
  }

  auto message = CefProcessMessage::Create(aegis::kAegisLifecycleMessage);
  auto args = message->GetArgumentList();
  args->SetString(0, aegis::kLifecycleContextReady);
  args->SetString(1, frame->GetURL());
  frame->SendProcessMessage(PID_BROWSER, message);
}

bool AegisApp::OnProcessMessageReceived(CefRefPtr<CefBrowser>,
                                        CefRefPtr<CefFrame> frame,
                                        CefProcessId source_process,
                                        CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_RENDERER_THREAD();
  AppendDebugLog("app: on_process_message_received renderer");
  if (source_process != PID_BROWSER || !message.get() ||
      message->GetName() != aegis::kAegisRequestMessage) {
    return false;
  }

  auto args = message->GetArgumentList();
  const int request_id = args->GetInt(0);
  const auto op = args->GetString(1).ToString();
  const auto body = args->GetString(2).ToString();

  std::string response;
  std::string error;
  if (DispatchRendererOperation(op, body, frame, &response, &error)) {
    SendRendererReply(frame, request_id, true, response);
  } else {
    SendRendererReply(frame, request_id, false, error);
  }
  return true;
}

void AegisApp::OnPrimaryBrowserCreated(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  if (!primary_browser_) {
    primary_browser_ = browser;
  }
#if defined(AEGIS_STANDALONE_APP)
  if (browser) {
    AegisSetBrowserHostAddress(browser->GetMainFrame()->GetURL().ToString());
    AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                       browser->IsLoading());
    AegisAttachBrowserToHostWindow(browser);
    AegisShowBrowserHostWindow();
  }
#endif
}

void AegisApp::OnLoadingStateChange(CefRefPtr<CefBrowser> browser, bool is_loading) {
  CEF_REQUIRE_UI_THREAD();
#if defined(AEGIS_STANDALONE_APP)
  if (browser) {
    AegisSetBrowserHostNavigationState(browser->CanGoBack(), browser->CanGoForward(),
                                       is_loading);
  }
#else
  (void)browser;
  (void)is_loading;
#endif
}

void AegisApp::OnAddressChange(CefRefPtr<CefBrowser>,
                               CefRefPtr<CefFrame> frame,
                               const CefString& url) {
  CEF_REQUIRE_UI_THREAD();
#if defined(AEGIS_STANDALONE_APP)
  if (frame && frame->IsMain()) {
    AegisSetBrowserHostAddress(url.ToString());
  }
#else
  (void)frame;
  (void)url;
#endif
}

void AegisApp::OnTitleChange(CefRefPtr<CefBrowser>,
                             const CefString& title) {
  CEF_REQUIRE_UI_THREAD();
#if defined(AEGIS_STANDALONE_APP)
  AegisSetBrowserHostTitle(title.ToString());
#else
  (void)title;
#endif
}

void AegisApp::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  if (primary_browser_ && browser &&
      primary_browser_->GetIdentifier() == browser->GetIdentifier()) {
    primary_browser_ = nullptr;
    request_context_ = nullptr;
#if defined(AEGIS_STANDALONE_APP)
    AegisRemoveRuntimeSession(runtime_session_paths_);
    AegisCloseBrowserHostWindow();
    AegisStopApplicationMessageLoop();
#endif
  }
}
