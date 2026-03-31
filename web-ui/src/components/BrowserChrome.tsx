import { colors } from "../tokens";
import { useChromeState } from "../hooks/useChromeState";
import { useNavigation } from "../hooks/useNavigation";
import { Toolbar } from "./Toolbar";
import { PageTransition } from "./PageTransition";

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

      {/* Content area — noVNC canvas or browser content renders here */}
      <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
        <PageTransition isLoading={state.isLoading} />
      </div>
    </div>
  );
}
