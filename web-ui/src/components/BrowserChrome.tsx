import { colors } from "../tokens";
import { useChromeState } from "../hooks/useChromeState";
import { useNavigation } from "../hooks/useNavigation";
import { Toolbar } from "./Toolbar";
import { PageTransition } from "./PageTransition";
import { RemoteViewport } from "./RemoteViewport";
import { TelemetryPanel } from "./TelemetryPanel";

export function BrowserChrome() {
  const chrome = useChromeState();
  const nav = useNavigation();
  const activeTab =
    chrome.tabs.find((tab) => tab.id === chrome.activeTabId) ?? chrome.tabs[0];

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        background: colors.windowBg,
        minWidth: 520,
        minHeight: 400,
      }}
    >
      <Toolbar
        tabs={chrome.tabs}
        activeTabId={chrome.activeTabId}
        title={activeTab?.title ?? "Aegis"}
        url={activeTab?.url ?? ""}
        canGoBack={activeTab?.canGoBack ?? false}
        canGoForward={activeTab?.canGoForward ?? false}
        isLoading={activeTab?.isLoading ?? false}
        onBack={nav.goBack}
        onForward={nav.goForward}
        onReload={nav.reload}
        onStop={nav.stop}
        onNavigate={nav.navigate}
        onCreateTab={nav.createTab}
        onActivateTab={nav.activateTab}
        onCloseTab={nav.closeTab}
      />

      <div className="browser-shell">
        <div style={{ flex: 1, position: "relative", overflow: "hidden", minHeight: 0 }}>
          <RemoteViewport />
          <PageTransition isLoading={activeTab?.isLoading ?? false} />
        </div>
        <TelemetryPanel />
      </div>
    </div>
  );
}
