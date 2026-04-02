import { colors } from "../tokens";
import { useChromeState } from "../hooks/useChromeState";
import { useNavigation } from "../hooks/useNavigation";
import { Toolbar } from "./Toolbar";
import { PageTransition } from "./PageTransition";
import { RemoteViewport } from "./RemoteViewport";
import { TelemetryPanel } from "./TelemetryPanel";

export function BrowserChrome() {
  const state = useChromeState();
  const nav = useNavigation();

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
        title={state.title}
        url={state.url}
        canGoBack={state.canGoBack}
        canGoForward={state.canGoForward}
        isLoading={state.isLoading}
        onBack={nav.goBack}
        onForward={nav.goForward}
        onReload={nav.reload}
        onStop={nav.stop}
        onNavigate={nav.navigate}
      />

      <div className="browser-shell">
        <div style={{ flex: 1, position: "relative", overflow: "hidden", minHeight: 0 }}>
          <RemoteViewport />
          <PageTransition isLoading={state.isLoading} />
        </div>
        <TelemetryPanel />
      </div>
    </div>
  );
}
