#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>

#include "aegis_native_mac.h"
#include "include/cef_application_mac.h"

#include <stdexcept>
#include <string>

@interface NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal;
@end

@implementation NSAlert (AegisSuppression)
- (NSModalResponse)aegis_runModal {
  return NSModalResponseCancel;
}
@end

void AegisInstallModalAlertSuppression() {
  static dispatch_once_t once_token;
  dispatch_once(&once_token, ^{
    Method original = class_getInstanceMethod([NSAlert class], @selector(runModal));
    Method replacement = class_getInstanceMethod([NSAlert class], @selector(aegis_runModal));
    method_exchangeImplementations(original, replacement);
  });
}

void AegisConfigureCefSettings(const AegisCefBootstrapOptions& options,
                               CefSettings* settings) {
  if (settings == nullptr) {
    return;
  }
#if !defined(CEF_USE_SANDBOX)
  settings->no_sandbox = true;
#endif
  settings->windowless_rendering_enabled = options.headless;
  settings->command_line_args_disabled = false;
  settings->external_message_pump = options.external_message_pump;

  if (!options.browser_subprocess_path.empty()) {
    CefString(&settings->browser_subprocess_path) = options.browser_subprocess_path;
  }
  if (!options.framework_dir_path.empty()) {
    CefString(&settings->framework_dir_path) = options.framework_dir_path;
  }
  if (!options.main_bundle_path.empty()) {
    CefString(&settings->main_bundle_path) = options.main_bundle_path;
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
  if (options.headless) {
    AegisInstallModalAlertSuppression();
  }
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

#include "aegis_browser_host_mac.inc"
