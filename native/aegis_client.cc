#include "aegis_client.h"

#include <cstdlib>
#include <fstream>
#include <sstream>

#include "include/base/cef_callback.h"
#include "include/cef_app.h"
#include "include/cef_parser.h"
#include "include/wrapper/cef_helpers.h"

namespace {

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

class AegisResourceRequestHandler : public CefResourceRequestHandler {
 public:
  explicit AegisResourceRequestHandler(AegisClientDelegate* delegate)
      : delegate_(delegate) {}

  ReturnValue OnBeforeResourceLoad(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefRequest> request,
                                   CefRefPtr<CefCallback>) override {
    return delegate_ ? delegate_->OnBeforeResourceLoad(browser, frame, request)
                     : RV_CONTINUE;
  }

  void OnResourceLoadComplete(CefRefPtr<CefBrowser> browser,
                              CefRefPtr<CefFrame> frame,
                              CefRefPtr<CefRequest> request,
                              CefRefPtr<CefResponse> response,
                              URLRequestStatus status,
                              int64_t) override {
    if (delegate_) {
      delegate_->OnResourceLoadComplete(browser, frame, request, response, status);
    }
  }

  void OnResourceRedirect(CefRefPtr<CefBrowser> browser,
                          CefRefPtr<CefFrame> frame,
                          CefRefPtr<CefRequest> request,
                          CefRefPtr<CefResponse> response,
                          CefString& new_url) override {
    if (delegate_) {
      delegate_->OnResourceRedirect(browser, frame, request, response, new_url);
    }
  }

 private:
  AegisClientDelegate* delegate_;

  IMPLEMENT_REFCOUNTING(AegisResourceRequestHandler);
};

std::string DataUri(const std::string& data) {
  return "data:text/html;base64," +
         CefURIEncode(CefBase64Encode(data.data(), data.size()), false)
             .ToString();
}

}  // namespace

AegisClient::AegisClient(bool headless, AegisClientDelegate* delegate)
    : headless_(headless), delegate_(delegate) {}

void AegisClient::OnTitleChange(CefRefPtr<CefBrowser> browser,
                                const CefString& title) {
  CEF_REQUIRE_UI_THREAD();
  if (headless_) {
    return;
  }
  if (delegate_) {
    delegate_->OnTitleChange(browser, title);
  }
}

void AegisClient::OnAddressChange(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefFrame> frame,
                                  const CefString& url) {
  CEF_REQUIRE_UI_THREAD();
  if (!delegate_) {
    return;
  }
  delegate_->OnAddressChange(browser, frame, url);
}

void AegisClient::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog("client: on_after_created");
  if (delegate_) {
    delegate_->OnPrimaryBrowserCreated(browser);
  }
}

bool AegisClient::OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                                 CefRefPtr<CefFrame> frame,
                                 CefRefPtr<CefRequest> request,
                                 bool,
                                 bool) {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog("client: on_before_browse");
  if (delegate_) {
    delegate_->OnBeforeBrowse(browser, frame, request);
  }
  return false;
}

bool AegisClient::DoClose(CefRefPtr<CefBrowser>) {
  CEF_REQUIRE_UI_THREAD();
  is_closing_ = true;
  return false;
}

void AegisClient::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog("client: on_before_close");
  if (delegate_) {
    delegate_->OnBeforeClose(browser);
  }
  is_closing_ = true;
}

void AegisClient::OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                                       bool isLoading,
                                       bool,
                                       bool) {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog(std::string("client: on_loading_state_change loading=") +
                 (isLoading ? "true" : "false"));
  if (delegate_) {
    delegate_->OnLoadingStateChange(browser, isLoading);
  }
}

void AegisClient::OnLoadEnd(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            int httpStatusCode) {
  CEF_REQUIRE_UI_THREAD();
  AppendDebugLog(std::string("client: on_load_end status=") +
                 std::to_string(httpStatusCode));
  if (delegate_) {
    delegate_->OnLoadEnd(browser, frame, httpStatusCode);
  }
}

CefRefPtr<CefResourceRequestHandler> AegisClient::GetResourceRequestHandler(
    CefRefPtr<CefBrowser>,
    CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest>,
    bool,
    bool,
    const CefString&,
    bool&) {
  if (!delegate_) {
    return nullptr;
  }
  return new AegisResourceRequestHandler(delegate_);
}

void AegisClient::OnLoadError(CefRefPtr<CefBrowser> browser,
                              CefRefPtr<CefFrame> frame,
                              ErrorCode errorCode,
                              const CefString& errorText,
                              const CefString& failedUrl) {
  CEF_REQUIRE_UI_THREAD();
  if (errorCode == ERR_ABORTED) {
    return;
  }

  if (delegate_) {
    delegate_->OnLoadError(browser, frame, errorCode, errorText, failedUrl);
  }

  std::stringstream html;
  html << "<html><body><h2>Failed to load " << std::string(failedUrl)
       << "</h2><p>" << std::string(errorText) << " (" << errorCode
       << ")</p></body></html>";
  frame->LoadURL(DataUri(html.str()));
}

void AegisClient::GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) {
  rect = CefRect(0, 0, 1280, 800);
}

void AegisClient::OnPaint(CefRefPtr<CefBrowser>,
                          PaintElementType,
                          const RectList&,
                          const void*,
                          int,
                          int) {}
