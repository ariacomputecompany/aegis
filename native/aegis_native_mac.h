#ifndef AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_
#define AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_

#include <string>

#include "include/cef_app.h"
#include "include/cef_browser.h"
#include "include/internal/cef_types.h"

struct AegisCefBootstrapOptions {
  bool headless = false;
  bool external_message_pump = false;
  bool initialize_browser_host_application = false;
  std::string browser_subprocess_path;
  std::string framework_dir_path;
  std::string main_bundle_path;
  std::string resources_dir_path;
  std::string locales_dir_path;
  std::string root_cache_path;
  std::string cache_path;
};

std::string AegisStandaloneRootCachePath();
std::string AegisStandaloneCachePath();
void AegisInstallModalAlertSuppression();
void AegisInitializeBrowserHostApplication();
void AegisConfigureCefSettings(const AegisCefBootstrapOptions& options,
                               CefSettings* settings);
bool AegisExecuteProcessAndInitialize(const CefMainArgs& main_args,
                                      const AegisCefBootstrapOptions& options,
                                      CefRefPtr<CefApp> app,
                                      int* subprocess_exit_code,
                                      std::string* error);
CefWindowHandle AegisCreateBrowserHostView(const std::string& title,
                                           int width,
                                           int height);
void AegisShowBrowserHostWindow();
void AegisSetBrowserHostTitle(const std::string& title);
void AegisSetBrowserHostAddress(const std::string& url);
void AegisSetBrowserHostNavigationState(bool can_go_back,
                                        bool can_go_forward,
                                        bool is_loading);
void AegisAttachBrowserToHostWindow(CefRefPtr<CefBrowser> browser);
void AegisCloseBrowserHostWindow();
void AegisScheduleCefMessagePumpWork(int64_t delay_ms);
bool AegisRunScheduledCefMessagePumpWorkIfDue();
int64_t AegisNextScheduledCefWorkDelayMs();
void AegisResetCefMessagePumpScheduler();
void AegisRunApplicationMessageLoop();
void AegisStopApplicationMessageLoop();
void AegisPumpBrowserHostWindow();
bool AegisBrowserHostWindowCloseRequested();

#endif  // AEGIS_NATIVE_AEGIS_NATIVE_MAC_H_
